//! Which mounted volumes can be a sync hub, what KIND each is, an identity
//! that survives a rename and a remount, and the hubs the user has chosen.
//!
//! Classification is one `statfs` per entry of `/Volumes` (and, for a local
//! disk, one `getattrlist` for its volume UUID) — no `diskutil` spawn, no TCC
//! prompt. Measured on macOS 27.0 with two USB SSDs: `ATTR_VOL_UUID` equals
//! `diskutil info`'s `VolumeUUID`, and `MNT_REMOVABLE` ("storage which can be
//! removed from the system by the user", sys/mount.h) is set on both, while
//! `diskutil`'s own `Removable` key says `false` for them — so the mount flag
//! is the one that means what the user means by "a drive I plug in".
//!
//! * **removable** — `MNT_REMOVABLE`. The only kind auto-detect ever picks.
//! * **network** — `f_fstypename` is smbfs/afpfs/nfs/webdav. Listed and
//!   selectable, never auto-picked: a share that is always mounted would make
//!   auto-detect ambiguous on every machine that has one (which is what the old
//!   hardcoded username in the denylist was papering over).
//! * **internal** — any other local mount under `/Volumes` (a second internal
//!   volume, a disk image). Selectable, never auto-picked.
//!
//! What is never a hub: an entry that is not a mount point (a stale directory
//! left in `/Volumes`), a symlink (`Macintosh HD -> /`), the root filesystem,
//! a `MNT_DONTBROWSE` mount (Time Machine's), and Time Machine's names.
//!
//! The chosen hubs live in `$XDG_CONFIG_HOME/worktrees/sync-hubs.json`, owned
//! by the backend and written only with `fsx::write_atomic`. The CLI reads it
//! too, BELOW `[sync] hub` (see `sync::resolve_hub`), so the app and the
//! terminal agree on which drive "the hub" is. A chosen hub that is not
//! mounted is an error naming it — never a reason to fall back to whatever
//! other drive happens to be plugged in.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE: &str = "sync-hubs.json";
const VERSION: u32 = 1;

/// Never a hub, by name: Time Machine's mounts. (The system disk is a symlink
/// and the root filesystem; both are refused by the flags as well.)
const NEVER: &[&str] = &["Macintosh HD", ".timemachine", "com.apple.TimeMachine.localsnapshots"];

const NETWORK_FS: &[&str] = &["smbfs", "afpfs", "nfs", "webdav"];

// mount flags (sys/mount.h). Spelled out because libc does not export
// MNT_REMOVABLE, and these values are ABI.
const MNT_REMOVABLE: u32 = 0x0000_0200;
const MNT_ROOTFS: u32 = 0x0000_4000;
const MNT_DONTBROWSE: u32 = 0x0010_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HubKind {
    Removable,
    Internal,
    Network,
    /// A path the user configured (`[sync] hub`, `$WORKTREES_SYNC_HUB`) that is
    /// not one of the volumes under `/Volumes`.
    Folder,
}

impl HubKind {
    pub fn label(self) -> &'static str {
        match self {
            HubKind::Removable => "removable",
            HubKind::Internal => "internal",
            HubKind::Network => "network",
            HubKind::Folder => "folder",
        }
    }
}

/// What the OS says about one entry of `/Volumes`. The pure input to
/// `classify` — tests build these by hand, so nothing in a test reads a real
/// volume.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VolStat {
    pub name: String,
    pub path: PathBuf,
    pub symlink: bool,
    /// `f_mntonname` is this path: a real mount, not a directory left behind.
    pub mount_point: bool,
    pub fstype: String,
    pub flags: u32,
    /// `f_mntfromname`: `/dev/disk4s1`, `//user@host/share`, `host:/export`.
    pub from: String,
    /// `ATTR_VOL_UUID`, for a local volume.
    pub uuid: Option<String>,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

/// A volume that can be a hub.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Volume {
    /// `uuid:<VOLUME-UUID>` for a local disk, `smb://host/share` (scheme from
    /// the filesystem, user stripped) for a share, `path:<path>` otherwise.
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub kind: HubKind,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

/// A share's identity from `f_mntfromname`. The user part is dropped: the same
/// share mounted by another account (or after a password change that put a
/// `;AUTH=` in it) is the same hub.
pub fn share_id(fstype: &str, from: &str) -> String {
    let scheme = match fstype {
        "smbfs" => "smb",
        "afpfs" => "afp",
        "nfs" => "nfs",
        "webdav" => "webdav",
        other => other,
    };
    if fstype == "webdav" {
        // Already a URL. Drop any userinfo.
        if let Some((sch, rest)) = from.split_once("://") {
            let rest = match rest.split_once('/') {
                Some((auth, path)) => format!("{}/{path}", auth.rsplit('@').next().unwrap_or(auth)),
                None => rest.rsplit('@').next().unwrap_or(rest).to_string(),
            };
            return format!("{sch}://{rest}");
        }
        return format!("webdav://{from}");
    }
    if fstype == "nfs" {
        // `host:/export/path`
        if let Some((host, path)) = from.split_once(':') {
            return format!("nfs://{host}{path}");
        }
    }
    let body = from.trim_start_matches('/');
    let (auth, path) = body.split_once('/').unwrap_or((body, ""));
    let host = auth.rsplit('@').next().unwrap_or(auth);
    if path.is_empty() {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}/{path}")
    }
}

/// Is this entry a hub candidate, and what kind? Pure.
pub fn classify(v: &VolStat) -> Option<Volume> {
    if v.symlink || !v.mount_point || NEVER.contains(&v.name.as_str()) {
        return None;
    }
    if v.flags & (MNT_ROOTFS | MNT_DONTBROWSE) != 0 {
        return None;
    }
    let (kind, id) = if NETWORK_FS.contains(&v.fstype.as_str()) {
        (HubKind::Network, share_id(&v.fstype, &v.from))
    } else {
        let kind = if v.flags & MNT_REMOVABLE != 0 { HubKind::Removable } else { HubKind::Internal };
        let id = match &v.uuid {
            Some(u) if !u.is_empty() => format!("uuid:{}", u.to_uppercase()),
            _ => format!("path:{}", v.path.display()),
        };
        (kind, id)
    };
    Some(Volume {
        id,
        name: v.name.clone(),
        path: v.path.clone(),
        kind,
        free_bytes: v.free_bytes,
        total_bytes: v.total_bytes,
    })
}

/// Name-sorted candidates from raw stats.
pub fn candidates_from(stats: &[VolStat]) -> Vec<Volume> {
    let mut v: Vec<Volume> = stats.iter().filter_map(classify).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// The test seam for every caller above core: a JSON array of `VolStat` in a
/// file named by `$WORKTREES_SYNC_VOLUMES`, read INSTEAD of `/Volumes`. The bats
/// suite and the api's tests set it, so no test lists a real drive.
const FAKE_ENV: &str = "WORKTREES_SYNC_VOLUMES";

/// Every hub candidate mounted right now.
pub fn volumes() -> Vec<Volume> {
    if let Ok(p) = std::env::var(FAKE_ENV) {
        if !p.is_empty() {
            let stats: Vec<VolStat> = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            return candidates_from(&stats);
        }
    }
    candidates_from(&read_volumes_dir(Path::new("/Volumes")))
}

fn read_volumes_dir(dir: &Path) -> Vec<VolStat> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let symlink =
            std::fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if symlink {
            out.push(VolStat { name, path: p, symlink, ..Default::default() });
            continue;
        }
        if let Some(mut s) = probe(&p) {
            s.name = name;
            out.push(s);
        }
    }
    out
}

/// Bytes free / total for a path's filesystem (`statfs`), for any hub.
pub fn space(path: &Path) -> (Option<u64>, Option<u64>) {
    probe(path).map(|s| (s.free_bytes, s.total_bytes)).unwrap_or((None, None))
}

#[cfg(target_os = "macos")]
fn cstr(buf: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(target_os = "macos")]
fn probe(p: &Path) -> Option<VolStat> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(p.as_os_str().as_bytes()).ok()?;
    // SAFETY: statfs fills a caller-owned struct; zeroed is a valid start.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let on = cstr(&st.f_mntonname);
    let fstype = cstr(&st.f_fstypename);
    let bs = st.f_bsize as u64;
    let local = !NETWORK_FS.contains(&fstype.as_str());
    Some(VolStat {
        name: String::new(),
        path: p.to_path_buf(),
        symlink: false,
        mount_point: Path::new(&on) == p,
        fstype,
        flags: st.f_flags,
        from: cstr(&st.f_mntfromname),
        uuid: if local { vol_uuid(&c) } else { None },
        free_bytes: Some(st.f_bavail.saturating_mul(bs)),
        total_bytes: Some(st.f_blocks.saturating_mul(bs)),
    })
}

/// `getattrlist(ATTR_VOL_UUID)` — the volume's own UUID, the same value
/// `diskutil info` prints as VolumeUUID. Survives a rename and a remount.
#[cfg(target_os = "macos")]
fn vol_uuid(path: &std::ffi::CStr) -> Option<String> {
    #[repr(C, packed(4))]
    struct Buf {
        len: u32,
        uuid: [u8; 16],
    }
    let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
    al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    al.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID;
    let mut buf = Buf { len: 0, uuid: [0; 16] };
    // SAFETY: the buffer is sized for exactly the one attribute asked for.
    let r = unsafe {
        libc::getattrlist(
            path.as_ptr(),
            &mut al as *mut libc::attrlist as *mut libc::c_void,
            &mut buf as *mut Buf as *mut libc::c_void,
            std::mem::size_of::<Buf>(),
            0,
        )
    };
    let u = buf.uuid;
    if r != 0 || u.iter().all(|b| *b == 0) {
        return None;
    }
    Some(format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        u[0], u[1], u[2], u[3], u[4], u[5], u[6], u[7], u[8], u[9], u[10], u[11], u[12], u[13], u[14], u[15]
    ))
}

/// Not macOS: no `/Volumes` convention to classify. A directory there is
/// listed as an internal volume addressed by its path.
#[cfg(not(target_os = "macos"))]
fn probe(p: &Path) -> Option<VolStat> {
    if !p.is_dir() {
        return None;
    }
    Some(VolStat {
        name: String::new(),
        path: p.to_path_buf(),
        mount_point: true,
        ..Default::default()
    })
}

// ── auto-detect ─────────────────────────────────────────────────────────────

/// The CLI's last resort: exactly ONE removable volume. Network shares and
/// internal volumes are never auto-picked.
pub fn auto_pick(vols: &[Volume]) -> Result<PathBuf, String> {
    let removable: Vec<&Volume> = vols.iter().filter(|v| v.kind == HubKind::Removable).collect();
    match removable.len() {
        1 => Ok(removable[0].path.clone()),
        0 => {
            let others = vols.iter().filter(|v| v.kind != HubKind::Removable).count();
            if others > 0 {
                Err(format!(
                    "no removable drive mounted ({others} other volume{} — pick one with `worktrees sync hubs --choose NAME` or pass --hub PATH)",
                    if others == 1 { "" } else { "s" }
                ))
            } else {
                Err("no external volume mounted; plug in the SSD or pass --hub PATH".to_string())
            }
        }
        _ => {
            let list = removable
                .iter()
                .map(|v| format!("  {}", v.path.display()))
                .collect::<Vec<_>>()
                .join("\n");
            Err(format!(
                "multiple removable drives:\n{list}\nambiguous — choose one with `worktrees sync hubs --choose NAME`, or pass --hub PATH"
            ))
        }
    }
}

// ── the chosen/remembered file ──────────────────────────────────────────────

/// One project a remembered hub held when it was last read — the manifest
/// brief `sync status` lists. The dashboard shows these, dimmed, while the hub
/// is not connected.
pub type KnownProject = crate::sync::ManifestBrief;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Remembered {
    pub id: String,
    pub name: String,
    pub kind: HubKind,
    /// Where it was mounted last time. A display hint only: a hub is FOUND by
    /// `id`, never by this path (a second drive can be mounted at it).
    pub path: String,
    #[serde(default)]
    pub last_seen: i64,
    #[serde(default)]
    pub projects: Vec<KnownProject>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HubsFile {
    #[serde(default)]
    pub version: u32,
    /// Ordered ids. The FIRST is the primary hub — the CLI's, and a pull's
    /// default; the app ticks all of them for a push.
    #[serde(default)]
    pub chosen: Vec<String>,
    #[serde(default)]
    pub hubs: Vec<Remembered>,
}

impl HubsFile {
    pub fn remembered(&self, id: &str) -> Option<&Remembered> {
        self.hubs.iter().find(|h| h.id == id)
    }

    /// Record a volume (on choose, or on a read of a connected remembered
    /// hub). `projects: None` keeps the last-known list.
    pub fn remember(&mut self, v: &Volume, now: i64, projects: Option<Vec<KnownProject>>) {
        let path = v.path.to_string_lossy().into_owned();
        match self.hubs.iter_mut().find(|h| h.id == v.id) {
            Some(h) => {
                h.name = v.name.clone();
                h.kind = v.kind;
                h.path = path;
                h.last_seen = now;
                if let Some(p) = projects {
                    h.projects = p;
                }
            }
            None => self.hubs.push(Remembered {
                id: v.id.clone(),
                name: v.name.clone(),
                kind: v.kind,
                path,
                last_seen: now,
                projects: projects.unwrap_or_default(),
            }),
        }
    }

    pub fn forget(&mut self, id: &str) -> bool {
        let before = (self.hubs.len(), self.chosen.len());
        self.hubs.retain(|h| h.id != id);
        self.chosen.retain(|c| c != id);
        before != (self.hubs.len(), self.chosen.len())
    }
}

pub fn path() -> PathBuf {
    crate::profile::config_root_pub().join(FILE)
}

/// Lenient: a missing or unreadable file is "nothing chosen".
pub fn read_at(p: &Path) -> HubsFile {
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn read() -> HubsFile {
    read_at(&path())
}

pub fn write_at(p: &Path, f: &HubsFile) -> Result<(), String> {
    let mut f = f.clone();
    f.version = VERSION;
    let body = serde_json::to_vec_pretty(&f).map_err(|e| e.to_string())?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    crate::fsx::write_atomic(p, &body, None).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

/// The real file. Refused when a test forbade real writes (the registry's
/// guard, `registry::forbid_real_writes`): a test engine must use `write_at`.
fn write(f: &HubsFile) -> Result<(), String> {
    if crate::registry::real_writes_forbidden() {
        return Err(format!(
            "refused: a test reached the user's real {FILE} — write a temp file with `write_at`"
        ));
    }
    write_at(&path(), f)
}

/// Read-modify-write under one process-wide lock.
pub fn edit_at<T>(p: &Path, f: impl FnOnce(&mut HubsFile) -> Result<T, String>) -> Result<T, String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut file = read_at(p);
    let before = file.clone();
    let out = f(&mut file)?;
    if file != before {
        if p == path() && crate::registry::real_writes_forbidden() {
            return Err(format!(
                "refused: a test reached the user's real {FILE} — write a temp file with `edit_at`"
            ));
        }
        write_at(p, &file)?;
    }
    Ok(out)
}

pub fn edit<T>(f: impl FnOnce(&mut HubsFile) -> Result<T, String>) -> Result<T, String> {
    edit_at(&path(), f)
}

// ── resolution ──────────────────────────────────────────────────────────────

/// The primary chosen hub, if one is chosen: `Ok(path)` when it is mounted,
/// `Err` naming it when it is not. `None` = nothing chosen.
pub fn primary(file: &HubsFile, vols: &[Volume]) -> Option<Result<PathBuf, String>> {
    let id = file.chosen.first()?;
    Some(match vols.iter().find(|v| &v.id == id) {
        Some(v) => Ok(v.path.clone()),
        None => {
            let name = file.remembered(id).map(|h| h.name.as_str()).unwrap_or(id.as_str());
            Err(format!(
                "the chosen hub '{name}' is not connected — plug it in, choose another with \
                 `worktrees sync hubs --choose NAME`, or pass --hub PATH"
            ))
        }
    })
}

/// One row of `worktrees sync hubs` and of the dashboard: a connected volume
/// or a remembered hub, never both for one id.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HubRow {
    pub id: String,
    pub name: String,
    pub kind: HubKind,
    /// Mounted path; `None` when not connected.
    pub path: Option<String>,
    pub connected: bool,
    /// Position in the chosen list (0 = primary), or `None`.
    pub chosen: Option<usize>,
    /// The `[sync] hub` / `$WORKTREES_SYNC_HUB` path — it overrides the choice.
    pub configured: bool,
    /// What auto-detect would pick (with nothing chosen or configured).
    pub auto: bool,
    pub last_seen: Option<i64>,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

/// Pure: connected volumes + remembered hubs + a configured path → rows.
/// Order: chosen (in chosen order), then connected, then remembered.
pub fn rows(vols: &[Volume], file: &HubsFile, configured: Option<&Path>) -> Vec<HubRow> {
    let auto = auto_pick(vols).ok();
    let rank = |id: &str| file.chosen.iter().position(|c| c == id);
    let mut out: Vec<HubRow> = vols
        .iter()
        .map(|v| HubRow {
            id: v.id.clone(),
            name: v.name.clone(),
            kind: v.kind,
            path: Some(v.path.to_string_lossy().into_owned()),
            connected: true,
            chosen: rank(&v.id),
            configured: configured == Some(v.path.as_path()),
            auto: auto.as_deref() == Some(v.path.as_path()),
            last_seen: file.remembered(&v.id).map(|h| h.last_seen),
            free_bytes: v.free_bytes,
            total_bytes: v.total_bytes,
        })
        .collect();
    if let Some(c) = configured {
        if !vols.iter().any(|v| v.path == c) {
            let (free, total) = if c.is_dir() { space(c) } else { (None, None) };
            out.push(HubRow {
                id: format!("path:{}", c.display()),
                name: c.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| c.display().to_string()),
                kind: HubKind::Folder,
                path: c.is_dir().then(|| c.to_string_lossy().into_owned()),
                connected: c.is_dir(),
                chosen: None,
                configured: true,
                auto: false,
                last_seen: None,
                free_bytes: free,
                total_bytes: total,
            });
        }
    }
    for h in &file.hubs {
        if out.iter().any(|r| r.id == h.id) {
            continue;
        }
        out.push(HubRow {
            id: h.id.clone(),
            name: h.name.clone(),
            kind: h.kind,
            path: None,
            connected: false,
            chosen: rank(&h.id),
            configured: false,
            auto: false,
            last_seen: Some(h.last_seen),
            free_bytes: None,
            total_bytes: None,
        });
    }
    // A chosen id that was never remembered (hand-edited file) still shows.
    for id in &file.chosen {
        if !out.iter().any(|r| &r.id == id) {
            out.push(HubRow {
                id: id.clone(),
                name: id.clone(),
                kind: HubKind::Folder,
                path: None,
                connected: false,
                chosen: rank(id),
                configured: false,
                auto: false,
                last_seen: None,
                free_bytes: None,
                total_bytes: None,
            });
        }
    }
    out.sort_by_key(|r| (r.chosen.unwrap_or(usize::MAX), !r.connected));
    out
}

/// Find a hub by what a user or client typed: an exact id, then a connected
/// volume's name, then its path. Only CONNECTED volumes are choosable by name
/// or path (a remembered one is chosen by id).
pub fn find<'a>(vols: &'a [Volume], key: &str) -> Option<&'a Volume> {
    vols.iter()
        .find(|v| v.id == key)
        .or_else(|| vols.iter().find(|v| v.name == key))
        .or_else(|| vols.iter().find(|v| v.path.as_os_str() == key))
}

/// The path of a hub a client named by id: a connected volume, or the
/// configured folder. Anything else — a remembered hub that is not mounted, an
/// id nobody listed — is refused with the reason, so a client can never steer
/// a sync at a path of its choosing.
pub fn path_for_id(vols: &[Volume], file: &HubsFile, configured: Option<&Path>, id: &str) -> Result<PathBuf, String> {
    if let Some(v) = vols.iter().find(|v| v.id == id) {
        return Ok(v.path.clone());
    }
    if let Some(c) = configured {
        if id == format!("path:{}", c.display()) {
            return if c.is_dir() {
                Ok(c.to_path_buf())
            } else {
                Err(format!("the configured hub {} is not there", c.display()))
            };
        }
    }
    match file.remembered(id) {
        Some(h) => Err(format!("hub '{}' is not connected", h.name)),
        None => Err(format!("unknown hub: {id}")),
    }
}

// ── the dashboard ───────────────────────────────────────────────────────────

/// One project on one hub, and whether this machine has it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HubProjectView {
    pub name: String,
    pub local_root: String,
    pub pushed_at: i64,
    pub host: String,
    /// Root of the registered project that syncs to this slot (same name), if
    /// this machine has one. Its `LocalProject` row carries the local state.
    pub here: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HubView {
    #[serde(flatten)]
    pub row: HubRow,
    pub projects: Vec<HubProjectView>,
    /// `projects` is the LAST-KNOWN list (the hub is not connected).
    pub stale: bool,
}

/// A registered project on this machine, as the dashboard compares it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocalProject {
    pub name: String,
    pub root: String,
    #[serde(flatten)]
    pub state: crate::sync::LocalState,
    /// Non-null ⇒ this checkout arrived on a hub; it cannot be pushed.
    pub hub_copy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Dashboard {
    pub hubs: Vec<HubView>,
    pub local: Vec<LocalProject>,
    /// The reason a hub-dependent action has no default (`resolve_hub`'s
    /// error), or null when there is one.
    pub default_error: Option<String>,
}

/// Assemble the dashboard. `roots` are the registered projects (the caller's
/// scope); `file` is where the chosen/remembered hubs live. Every connected
/// hub that is REMEMBERED has its project list snapshotted into the file, so
/// it can be shown while unplugged; nothing else is written.
pub fn dashboard_in(
    vols: &[Volume],
    file: &Path,
    configured: Option<&Path>,
    roots: &[String],
    now: i64,
) -> Dashboard {
    let base = |r: &str| Path::new(r).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let local: Vec<LocalProject> = roots
        .iter()
        .map(|r| LocalProject {
            name: base(r),
            root: r.clone(),
            state: crate::sync::local_state(r),
            hub_copy: crate::sync::hub_copy_refusal(Path::new(r)),
        })
        .collect();
    let here = |name: &str| local.iter().find(|l| l.name == name).map(|l| l.root.clone());

    let mut hf = read_at(file);
    let before = hf.clone();
    let rows = rows(vols, &hf, configured);
    let mut hubs = Vec::new();
    for row in rows {
        let (projects, stale) = match &row.path {
            Some(p) if row.connected => {
                let list = crate::sync::hub_projects(Path::new(p));
                if let Some(v) = vols.iter().find(|v| v.id == row.id) {
                    if hf.remembered(&v.id).is_some() {
                        hf.remember(v, now, Some(list.clone()));
                    }
                }
                (list, false)
            }
            _ => (hf.remembered(&row.id).map(|h| h.projects.clone()).unwrap_or_default(), true),
        };
        let projects = projects
            .into_iter()
            .map(|m| HubProjectView { here: here(&m.name), name: m.name, local_root: m.local_root, pushed_at: m.pushed_at, host: m.host })
            .collect();
        hubs.push(HubView { row, projects, stale });
    }
    if hf != before {
        // A cache: losing it costs a dimmed list, never a sync. Not fatal.
        let _ = if file == path() { write(&hf) } else { write_at(file, &hf) };
    }
    let default_error = match configured {
        Some(_) => None,
        None => match primary(&hf, vols) {
            Some(r) => r.err(),
            None => auto_pick(vols).err(),
        },
    };
    Dashboard { hubs, local, default_error }
}

/// Make `keys` (ids, or connected volumes' names/paths) the chosen hubs, in
/// order. A key must name a connected volume or an already-remembered hub;
/// anything else is refused whole and nothing is written.
pub fn choose_in(file: &Path, vols: &[Volume], keys: &[String], now: i64) -> Result<Vec<String>, String> {
    edit_at(file, |f| {
        let mut ids = Vec::new();
        for k in keys {
            let id = if let Some(v) = find(vols, k) {
                f.remember(v, now, None);
                v.id.clone()
            } else if f.remembered(k).is_some() {
                k.clone()
            } else {
                return Err(format!("no such hub: {k} (plug it in first — `worktrees sync hubs` lists them)"));
            };
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        f.chosen = ids.clone();
        Ok(ids)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vs(name: &str, fstype: &str, flags: u32, from: &str, uuid: Option<&str>) -> VolStat {
        VolStat {
            name: name.into(),
            path: PathBuf::from(format!("/Volumes/{name}")),
            symlink: false,
            mount_point: true,
            fstype: fstype.into(),
            flags,
            from: from.into(),
            uuid: uuid.map(str::to_string),
            free_bytes: Some(1),
            total_bytes: Some(2),
        }
    }

    // flags measured on this Mac (macOS 27.0): a USB SSD, the root, Time Machine.
    const USB: u32 = 0x04a0_9218;
    const ROOT: u32 = 0x4480_d001;
    const TM: u32 = 0x0490_9080;

    #[test]
    fn classify_by_flags_and_fstype() {
        let ssd = classify(&vs("Extreme SSD", "apfs", USB, "/dev/disk41s1", Some("f04c7580-9f3d-4d31-be7c-b86550436e31"))).unwrap();
        assert_eq!(ssd.kind, HubKind::Removable);
        assert_eq!(ssd.id, "uuid:F04C7580-9F3D-4D31-BE7C-B86550436E31");

        let share = classify(&vs("projects", "smbfs", 0, "//david@nas._smb._tcp.local/projects", None)).unwrap();
        assert_eq!(share.kind, HubKind::Network);
        assert_eq!(share.id, "smb://nas._smb._tcp.local/projects");

        let internal = classify(&vs("Data2", "apfs", 0x0000_1000, "/dev/disk3s7", Some("AB"))).unwrap();
        assert_eq!(internal.kind, HubKind::Internal);

        assert!(classify(&vs("Root", "apfs", ROOT, "/dev/disk3s1s1", Some("X"))).is_none());
        assert!(classify(&vs(".timemachine", "apfs", TM, "/dev/disk3s5", Some("Y"))).is_none());
        assert!(classify(&vs("anything", "apfs", TM, "/dev/disk3s5", Some("Y"))).is_none(), "DONTBROWSE alone refuses");
        let mut stale = vs("Old", "apfs", USB, "/dev/disk3s1", Some("Z"));
        stale.mount_point = false;
        assert!(classify(&stale).is_none(), "a leftover directory is not a mount");
        let mut link = vs("Macintosh HD", "", 0, "", None);
        link.symlink = true;
        assert!(classify(&link).is_none());
        // no uuid → addressed by path
        assert_eq!(classify(&vs("Img", "hfs", USB, "/dev/disk9s1", None)).unwrap().id, "path:/Volumes/Img");
    }

    #[test]
    fn share_ids_drop_the_user() {
        assert_eq!(share_id("smbfs", "//david@host/share"), "smb://host/share");
        assert_eq!(share_id("smbfs", "//host/share"), "smb://host/share");
        assert_eq!(share_id("afpfs", "//david;AUTH=No%20User%20Authent@host/vol"), "afp://host/vol");
        assert_eq!(share_id("nfs", "nas:/export/home"), "nfs://nas/export/home");
        assert_eq!(share_id("webdav", "https://u@dav.example/x"), "https://dav.example/x");
    }

    #[test]
    fn auto_pick_takes_exactly_one_removable_and_never_a_share() {
        let ssd = vs("SSD", "apfs", USB, "/dev/disk4s1", Some("A"));
        let ssd2 = vs("SanDisk", "apfs", USB, "/dev/disk5s1", Some("B"));
        let share = vs("home", "smbfs", 0, "//u@nas/home", None);
        // the old denylist's case: a removable drive plus an always-mounted share
        assert_eq!(auto_pick(&candidates_from(&[ssd.clone(), share.clone()])).unwrap(), PathBuf::from("/Volumes/SSD"));
        let e = auto_pick(&candidates_from(&[ssd.clone(), ssd2])).unwrap_err();
        assert!(e.contains("multiple removable drives") && e.contains("--choose"), "{e}");
        let e = auto_pick(&candidates_from(&[share])).unwrap_err();
        assert!(e.contains("no removable drive") && e.contains("--choose"), "{e}");
        assert!(auto_pick(&[]).unwrap_err().contains("no external volume"));
    }

    #[test]
    fn a_chosen_hub_that_is_not_mounted_is_named_never_swapped() {
        let vols = candidates_from(&[vs("SanDisk", "apfs", USB, "/dev/disk5s1", Some("B"))]);
        let mut f = HubsFile::default();
        assert!(primary(&f, &vols).is_none());
        let ssd = classify(&vs("Extreme SSD", "apfs", USB, "/dev/disk4s1", Some("A"))).unwrap();
        f.remember(&ssd, 10, None);
        f.chosen = vec![ssd.id.clone()];
        let e = primary(&f, &vols).unwrap().unwrap_err();
        assert!(e.contains("'Extreme SSD' is not connected"), "{e}");
        // remounted elsewhere under a new name: found by id
        let mut moved = vs("Renamed", "apfs", USB, "/dev/disk7s1", Some("A"));
        moved.path = PathBuf::from("/Volumes/Renamed");
        let vols = candidates_from(&[moved]);
        assert_eq!(primary(&f, &vols).unwrap().unwrap(), PathBuf::from("/Volumes/Renamed"));
    }

    #[test]
    fn rows_put_chosen_first_and_keep_remembered_offline_hubs() {
        let a = vs("A", "apfs", USB, "/dev/disk4s1", Some("A"));
        let b = vs("B", "smbfs", 0, "//u@nas/b", None);
        let vols = candidates_from(&[a, b]);
        let mut f = HubsFile::default();
        let gone = Volume { id: "uuid:GONE".into(), name: "Old SSD".into(), path: "/Volumes/Old SSD".into(), kind: HubKind::Removable, free_bytes: None, total_bytes: None };
        f.remember(&gone, 5, Some(vec![KnownProject { name: "p".into(), local_root: "/r/p".into(), pushed_at: 4, host: "h".into() }]));
        f.chosen = vec!["smb://nas/b".into(), "uuid:GONE".into()];
        let r = rows(&vols, &f, None);
        let ids: Vec<&str> = r.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["smb://nas/b", "uuid:GONE", "uuid:A"]);
        assert!(r[0].connected && !r[1].connected && r[2].auto);
        assert_eq!(r[1].chosen, Some(1));
        assert!(path_for_id(&vols, &f, None, "uuid:GONE").unwrap_err().contains("not connected"));
        assert!(path_for_id(&vols, &f, None, "path:/etc").unwrap_err().contains("unknown hub"));
        assert_eq!(path_for_id(&vols, &f, None, "uuid:A").unwrap(), PathBuf::from("/Volumes/A"));
    }

    #[test]
    fn the_file_round_trips_and_forget_clears_the_choice() {
        let d = std::env::temp_dir().join(format!("wt-synchub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join(FILE);
        assert_eq!(read_at(&p), HubsFile::default());
        let v = Volume { id: "uuid:A".into(), name: "A".into(), path: "/Volumes/A".into(), kind: HubKind::Removable, free_bytes: None, total_bytes: None };
        edit_at(&p, |f| {
            f.remember(&v, 1, None);
            f.chosen.push(v.id.clone());
            Ok(())
        })
        .unwrap();
        let back = read_at(&p);
        assert_eq!(back.version, VERSION);
        assert_eq!(back.chosen, ["uuid:A"]);
        assert!(edit_at(&p, |f| Ok(f.forget("uuid:A"))).unwrap());
        assert_eq!(read_at(&p).hubs.len(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }
}
