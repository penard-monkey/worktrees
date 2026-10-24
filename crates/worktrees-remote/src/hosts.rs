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
//! **A host whose keys the caller holds is kept as its record alone**
//! ([`HostStore::add_record`]): the iOS app's keys are Secure Enclave signers,
//! with no bytes to save. Its record carries `"key": "external"`, and the
//! store never opens, writes or deletes a key file for it. A desktop's record
//! has no `key` field, so [`HostStore::add`] writes the file it always wrote,
//! and a build that predates the field reads either kind (it ignores the
//! field, and finds no key file for an external host).
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
    hosts: Vec<Entry>,
}

/// One host as the file holds it: the record, and where its device key is.
/// `key` is the store's own bookkeeping, so it stays off [`HostRecord`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    #[serde(flatten)]
    rec: HostRecord,
    #[serde(default, skip_serializing_if = "KeyHome::is_file")]
    key: KeyHome,
}

/// Where a pairing's device key is kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KeyHome {
    /// `remote-keys/<id>.key`, written by [`HostStore::add`]. Not written
    /// out, so a desktop's file is the one it always was.
    #[default]
    File,
    /// With the caller ([`HostStore::add_record`]).
    External,
}

impl KeyHome {
    fn is_file(&self) -> bool {
        *self == KeyHome::File
    }
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
        Ok(Hosts { chosen: f.chosen, hosts: f.hosts.into_iter().map(|e| e.rec).collect() })
    }

    /// The host `sel` names: its id, or else its name when exactly one host
    /// has it.
    pub fn get(&self, sel: &str) -> Result<HostRecord, String> {
        find(&self.read()?.hosts, sel).map(|e| e.rec.clone())
    }

    /// The device key of `rec`'s pairing. A host kept by [`add_record`]
    /// has none here, and no key file is opened for it.
    ///
    /// [`add_record`]: HostStore::add_record
    pub fn key_for(&self, rec: &HostRecord) -> Result<DeviceKey, String> {
        if !is_host_id(&rec.id) {
            return Err(format!("{:?} is not a host id", rec.id));
        }
        if self.read()?.hosts.iter().any(|e| e.rec.id == rec.id && e.key == KeyHome::External) {
            return Err(format!("{}'s device key is not kept here: the app that paired it holds it", rec.name));
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
            if f.hosts.iter().any(|h| h.rec.id == rec.id) {
                return Err(format!("a host with id {} is already kept", rec.id));
            }
            let keys = self.dir.join(KEYS_DIR);
            worktrees_core::fsx::make_dir(&keys).map_err(|e| format!("{}: {e}", keys.display()))?;
            fs::set_permissions(&keys, fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", keys.display()))?;
            key.save(&self.key_path(&rec.id))?;
            f.hosts.push(Entry { rec: rec.clone(), key: KeyHome::File });
            Ok(())
        })
    }

    /// Keep a pairing whose keys the CALLER holds — the iOS app's Secure
    /// Enclave signers, from `pair::pair_keys` — as its record alone: no key
    /// file is written, now or later, and [`forget`] deletes none. Under the
    /// same lock, in the same file and format as [`add`].
    ///
    /// An upsert by `rec.id`: a new host is added, and a host this call kept
    /// before is replaced in place. A host [`add`] kept, with its key file
    /// here, is refused: replacing it would leave that file behind.
    ///
    /// The caller keeps the keys first and the record second, as [`add`]
    /// does, so a record is never kept without its keys; and it deletes the
    /// keys itself, after [`forget`].
    ///
    /// [`add`]: HostStore::add
    /// [`forget`]: HostStore::forget
    pub fn add_record(&self, rec: &HostRecord) -> Result<(), String> {
        rec.check()?;
        self.locked(|f| {
            let entry = Entry { rec: rec.clone(), key: KeyHome::External };
            match f.hosts.iter().position(|h| h.rec.id == rec.id) {
                Some(i) if f.hosts[i].key == KeyHome::File => {
                    Err(format!("{} ({}) is kept with a key file on this device; forget it first", f.hosts[i].rec.name, rec.id))
                }
                Some(i) => {
                    f.hosts[i] = entry;
                    Ok(())
                }
                None if f.hosts.len() >= MAX_HOSTS => Err(format!("this device already keeps {MAX_HOSTS} hosts; forget one first")),
                None => {
                    f.hosts.push(entry);
                    Ok(())
                }
            }
        })
    }

    /// Forget `sel`: its record, its key file if this store wrote one, and
    /// the choice if it was chosen. The record that went.
    ///
    /// Only the store's own key file is deleted: a regular file, in the keys
    /// directory itself. A link at either is refused, and nothing is deleted
    /// or written. A host kept by [`add_record`] has no file here, and
    /// nothing under `remote-keys` is touched for it.
    ///
    /// [`add_record`]: HostStore::add_record
    pub fn forget(&self, sel: &str) -> Result<HostRecord, String> {
        self.locked(|f| {
            let gone = find(&f.hosts, sel)?.clone();
            let rec = gone.rec;
            f.hosts.retain(|h| h.rec.id != rec.id);
            if f.chosen.as_deref() == Some(rec.id.as_str()) {
                f.chosen = None;
            }
            if gone.key == KeyHome::File {
                self.remove_key_file(&rec).map_err(|e| format!("{e}; nothing was deleted, and {} is still kept", rec.name))?;
            }
            Ok(rec)
        })
    }

    /// Unlink `remote-keys/<id>.key` if it is this store's: a regular file in
    /// the keys directory itself. The directory is opened without following a
    /// link and the name is looked at and unlinked through that handle, so a
    /// link standing where either should be is never followed — swapped in
    /// after the look, a link is what gets unlinked, never its target.
    fn remove_key_file(&self, rec: &HostRecord) -> Result<(), String> {
        let keys = self.dir.join(KEYS_DIR);
        let path = self.key_path(&rec.id);
        let dir = match OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(&keys) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("{} is not this store's keys directory ({e})", keys.display())),
        };
        let name = std::ffi::CString::new(format!("{}.key", rec.id)).map_err(|_| format!("{:?} is not a host id", rec.id))?;
        // SAFETY: an all-zero `stat` is a valid value for fstatat to fill in.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: a live descriptor owned by `dir`, a NUL-terminated name, a
        // buffer of ours.
        if unsafe { libc::fstatat(dir.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            let e = std::io::Error::last_os_error();
            return match e.kind() {
                std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(format!("the device key {}: {e}", path.display())),
            };
        }
        if st.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err(format!("{} is not a key file this store wrote", path.display()));
        }
        // SAFETY: as above. Flags 0 unlinks a non-directory and never follows
        // the name.
        if unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(format!("the device key {}: {}", path.display(), std::io::Error::last_os_error()));
        }
        Ok(())
    }

    /// Keep `addr` as where host `id` now listens (§4.1, §4.7): called only
    /// once a pinned open to it has succeeded (`reach`). Re-read under the
    /// lock, so a record forgotten meanwhile is not brought back, and one
    /// paired again meanwhile — another pin — is not moved by a handshake
    /// made with the old one. The address passes the dial rule again.
    pub fn readdress(&self, id: &str, pin: &str, addr: &str) -> Result<HostRecord, String> {
        worktrees_core::netclass::parse_dial(addr).map_err(|e| e.to_string())?;
        self.locked(|f| {
            let h = &mut f.hosts.iter_mut().find(|h| h.rec.id == id).ok_or_else(|| format!("no host {id:?} is kept here any more"))?.rec;
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
                if !f.hosts.iter().any(|h| h.rec.id == id) {
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
            h.rec.check().map_err(|e| format!("{}: {e}", p.display()))?;
        }
        if let Some(c) = &f.chosen {
            if !f.hosts.iter().any(|h| &h.rec.id == c) {
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

fn find<'a>(hosts: &'a [Entry], sel: &str) -> Result<&'a Entry, String> {
    if let Some(h) = hosts.iter().find(|h| h.rec.id == sel) {
        return Ok(h);
    }
    let named: Vec<&Entry> = hosts.iter().filter(|h| h.rec.name == sel).collect();
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

    /// The file as every build before the `key` field wrote and read it: the
    /// v1 shape, records and nothing else.
    #[derive(Serialize, Deserialize)]
    struct OldFile1 {
        v: u32,
        #[serde(default)]
        chosen: Option<String>,
        #[serde(default)]
        hosts: Vec<HostRecord>,
    }

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    /// A desktop's pairing is written byte for byte as v1 always wrote it,
    /// and a file holding a record-only host still reads in a build that
    /// predates the `key` field (which ignores the field). A `key` this build
    /// does not know refuses the file.
    #[test]
    fn a_desktop_file_is_the_v1_it_was_and_a_record_only_one_still_reads_as_v1() {
        let d = dir("v1");
        let st = HostStore::new(&d);
        let (a, b) = (record("Mac mini"), record("iPhone's host"));
        st.add(&a, &DeviceKey::generate().unwrap()).unwrap();
        st.choose(Some(&a.id)).unwrap();
        let mut old = serde_json::to_vec_pretty(&OldFile1 { v: 1, chosen: Some(a.id.clone()), hosts: vec![a.clone()] }).unwrap();
        old.push(b'\n');
        assert_eq!(fs::read(d.join(HOSTS_FILE)).unwrap(), old, "byte for byte");
        st.add_record(&b).unwrap();
        let text = fs::read_to_string(d.join(HOSTS_FILE)).unwrap();
        assert_eq!(text.matches(r#""key": "external""#).count(), 1, "{text}");
        let read: OldFile1 = serde_json::from_str(&text).unwrap();
        assert_eq!((read.v, read.hosts), (1, vec![a.clone(), b.clone()]), "an older build reads both");
        fs::write(d.join(HOSTS_FILE), text.replace(r#""key": "external""#, r#""key": "keychain""#)).unwrap();
        assert!(st.list().is_err(), "an unknown key home is refused");
        let _ = fs::remove_dir_all(&d);
    }

    /// `add_record` keeps the record and writes no key — no file, no keys
    /// directory — and the store never touches a key path for that host:
    /// `key_for` refuses it without opening the file a stranger put where its
    /// key would be, and `forget` leaves that file. An upsert replaces a
    /// record-only host in place, and refuses one kept with a key file.
    #[test]
    fn a_record_is_kept_alone_and_no_key_path_is_touched_for_it() {
        let d = dir("record");
        let st = HostStore::new(&d);
        let (a, b) = (record("Mac mini"), record("Studio"));
        st.add_record(&a).unwrap();
        assert!(!d.join(KEYS_DIR).exists(), "no keys dir");
        assert_eq!(mode(&d.join(HOSTS_FILE)), 0o600);
        assert_eq!(st.get("Mac mini").unwrap(), a);
        assert!(st.add(&a, &DeviceKey::generate().unwrap()).is_err(), "kept already");
        assert!(!d.join(KEYS_DIR).exists(), "and no key written for it");

        let kb = DeviceKey::generate().unwrap();
        st.add(&b, &kb).unwrap();
        let renamed = HostRecord { name: "Mac mini (office)".into(), addr: "192.168.1.6:7461".into(), ..a.clone() };
        st.add_record(&renamed).unwrap();
        assert_eq!(st.list().unwrap().hosts, vec![renamed.clone(), b.clone()], "replaced in place");
        let before = fs::read(d.join(HOSTS_FILE)).unwrap();
        let e = st.add_record(&b).unwrap_err();
        assert!(e.contains("key file") && e.contains("forget it first"), "{e}");
        assert_eq!(fs::read(d.join(HOSTS_FILE)).unwrap(), before, "unchanged");
        assert!(st.add_record(&HostRecord { addr: "8.8.8.8:7461".into(), ..record("x") }).is_err(), "checked like any record");

        // A real key, 0600, where a's would be: never read, never deleted.
        let stray = st.key_path(&a.id);
        DeviceKey::generate().unwrap().save(&stray).unwrap();
        let e = st.key_for(&renamed).unwrap_err();
        assert!(e.contains("not kept here"), "{e}");
        assert_eq!(st.key_for(&b).unwrap().pin(), kb.pin(), "a desktop's key still loads");
        st.choose(Some(&a.id)).unwrap();
        assert_eq!(st.forget(&a.id).unwrap(), renamed);
        assert_eq!(st.list().unwrap(), Hosts { chosen: None, hosts: vec![b.clone()] });
        assert!(stray.exists(), "a file the store did not write is left");
        assert!(st.key_path(&b.id).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_record_only_host_counts_toward_the_most_a_device_keeps() {
        let d = dir("max");
        let st = HostStore::new(&d);
        for i in 0..MAX_HOSTS {
            st.add_record(&record(&format!("host {i}"))).unwrap();
        }
        let e = st.add_record(&record("one more")).unwrap_err();
        assert!(e.contains("forget one first"), "{e}");
        let first = st.list().unwrap().hosts[0].clone();
        st.add_record(&HostRecord { name: "renamed".into(), ..first }).unwrap();
        let _ = fs::remove_dir_all(&d);
    }

    /// `forget` deletes the store's own key file and nothing else: not
    /// through a link where the keys directory should be (the file it would
    /// reach is someone else's), and not a link where the key file should be.
    /// Either is refused with nothing deleted or written; once the link is
    /// gone, the host is forgotten.
    #[test]
    fn forget_never_deletes_through_a_link() {
        let d = dir("forget-link");
        let victim = dir("forget-link-victim");
        fs::create_dir_all(&victim).unwrap();
        let st = HostStore::new(&d);
        let a = record("Mac mini");
        st.add(&a, &DeviceKey::generate().unwrap()).unwrap();
        let name = format!("{}.key", a.id);
        fs::write(victim.join(&name), b"someone else's").unwrap();
        let keys = d.join(KEYS_DIR);
        let before = fs::read(d.join(HOSTS_FILE)).unwrap();

        fs::rename(&keys, d.join("keys-aside")).unwrap();
        std::os::unix::fs::symlink(&victim, &keys).unwrap();
        let e = st.forget(&a.id).unwrap_err();
        assert!(e.contains("not this store's keys directory") && e.contains("still kept"), "{e}");
        assert!(victim.join(&name).exists(), "nothing deleted through the link");
        assert_eq!(fs::read(d.join(HOSTS_FILE)).unwrap(), before, "nothing written");

        fs::remove_file(&keys).unwrap();
        fs::rename(d.join("keys-aside"), &keys).unwrap();
        fs::remove_file(keys.join(&name)).unwrap();
        std::os::unix::fs::symlink(victim.join(&name), keys.join(&name)).unwrap();
        let e = st.forget(&a.id).unwrap_err();
        assert!(e.contains("not a key file this store wrote"), "{e}");
        assert!(fs::symlink_metadata(keys.join(&name)).unwrap().file_type().is_symlink(), "the link is left");
        assert!(victim.join(&name).exists());
        assert_eq!(fs::read(d.join(HOSTS_FILE)).unwrap(), before);

        fs::remove_file(keys.join(&name)).unwrap();
        assert_eq!(st.forget(&a.id).unwrap(), a, "no key file left to delete");
        assert_eq!(fs::read(victim.join(&name)).unwrap(), b"someone else's");
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(&victim);
    }
}
