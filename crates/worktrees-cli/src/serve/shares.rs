//! `<config_dir>/shares.json` — what the owner has shared with guests, and
//! what guests have asked for (group chat, `docs/proposals/group-chat.md`
//! §2, §2.2, §3.3).
//!
//! **A share is attached to a LANE** (a project and a place) and holds a
//! **binding** to one harness session id at a time. The binding, not the
//! slug or the tmux session, says what a guest reads: those come back, and
//! `--name` survives `/clear`. A binding is `active` until serve sees a live
//! agent in the place with a DIFFERENT id; then it is `ended` for good, with
//! the transcript's byte length at that moment recorded, so a reader stops
//! there even if the old id is resumed and grows (PR C reads that offset).
//! No live agent is a pause, never an end. Only the owner's *Share again*
//! binds anything new, and the ended binding is kept beside it.
//!
//! **A share never outlives its lane.** The place's directory identity
//! (device and inode) is recorded at share time; a share whose project is no
//! longer registered, whose place directory is gone, or whose directory is
//! another one now (a later place with the same slug) is removed on serve's
//! start and on its poll ([`Shares::reconcile`]), whether the lane was
//! removed with serve up or down.
//!
//! **Requests** (§2.2): a guest's `share_request` grants nothing and reaches
//! no agent. A request for a hidden project's place, or for one that does not
//! exist, is stored as `silent` and answered exactly as a real one, so the
//! answer, the one-open-request rule and the rate all say nothing about what
//! is hidden. The console lists only the real ones.
//!
//! **The guest switch** (§2.2, *Show place names to guests*): the projects
//! hidden from `guest_places`, kept HERE, in host state, never in a
//! repository's `.worktrees.toml` — a cloned repository must not decide what
//! the owner's guests see. Default on: a project is shown unless listed.
//!
//! **Refused, never clobbered**, as `devices.json` is: the file is `0600`,
//! written whole through `fsx::write_atomic` under `shares.json.lock`; one
//! that does not parse, carries an unknown field or a newer version refuses
//! EVERY guest (the resolver asks [`Shares::read`] on every guest request)
//! and every write, and is left as it is for the person at the host.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use worktrees_core::authz::PrincipalId;

use super::servefile;

pub const FILE: &str = "shares.json";

/// The format this build reads and writes; a higher one is refused.
pub const VERSION: u32 = 1;

/// A request's note, at most, in bytes (§2.2: "a short limit").
pub const NOTE_MAX: usize = 280;

/// Share requests one guest may make per [`REQUEST_WINDOW`] (§2.2: "a
/// per-guest rate, so it can't be used as a doorbell"). Silent ones count.
pub const REQUESTS_PER_WINDOW: usize = 6;
pub const REQUEST_WINDOW: i64 = 3600;

/// An open request lapses after this, unanswered (§2.2: "until it expires or
/// David dismisses it").
pub const REQUEST_OPEN_FOR: i64 = 7 * 86_400;

/// The harnesses a share can bind (§3.2, D11): the ones with a transcript
/// reader.
pub const SHAREABLE: &[&str] = &["claude", "codex"];

// ── the record ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharesFile {
    pub version: u32,
    #[serde(default)]
    pub shares: Vec<Share>,
    #[serde(default)]
    pub requests: Vec<Request>,
    /// Registry keys of the projects whose place names guests do not see.
    #[serde(default)]
    pub hidden_projects: BTreeSet<String>,
}

impl Default for SharesFile {
    fn default() -> Self {
        SharesFile { version: VERSION, shares: Vec::new(), requests: Vec::new(), hidden_projects: BTreeSet::new() }
    }
}

/// The identity of a place's directory when it was shared: a later place
/// with the same slug is another directory and inherits nothing. Device and
/// inode are not enough on their own: Linux hands a removed directory's inode
/// to the next one made (measured on ubuntu-latest's ext4, where a remove and
/// a make under the same name kept it), so the directory's BIRTH time is part
/// of it too — `statx` on Linux, `st_birthtime` on macOS. A filesystem that
/// records no birth time falls back to device and inode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceId {
    pub dev: u64,
    pub ino: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub born_ns: Option<u64>,
}

impl PlaceId {
    pub fn of(dir: &Path) -> Option<PlaceId> {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(dir).ok().filter(|m| m.is_dir())?;
        let born_ns = m.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).and_then(|d| u64::try_from(d.as_nanos()).ok());
        Some(PlaceId { dev: m.dev(), ino: m.ino(), born_ns })
    }
}

/// Remove `dir` and make it again under the same name, as a removed place
/// and a new one with its slug would be — later than the old one's birth by
/// more than a filesystem's timestamp tick (ext4 stamps with a coarse clock,
/// a few milliseconds), so the birth times differ as they do between two
/// real places.
#[cfg(test)]
pub(crate) fn remake(dir: &Path) {
    fs::remove_dir_all(dir).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::create_dir_all(dir).unwrap();
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Share {
    pub id: String,
    /// The project's registry key.
    pub project: String,
    pub slug: String,
    pub place: PlaceId,
    pub created_at: i64,
    pub guests: Vec<ShareGuest>,
    /// The binding now: active, or ended and frozen until *Share again*.
    pub binding: Option<Binding>,
    /// Bindings a *Share again* replaced, oldest first: each still readable,
    /// frozen at its offset.
    #[serde(default)]
    pub ended: Vec<Binding>,
}

/// Where a guest stands on one share (§1 step 4): named by the owner,
/// asked to join (`share_join`), or let in (`share_allow`). Nothing but
/// `allowed` will read a transcript or a thread (PRs B and C).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Admission {
    Invited,
    Asked,
    Allowed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareGuest {
    pub principal: PrincipalId,
    pub state: Admission,
    pub since: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingState {
    Active,
    Ended,
}

/// One harness session a share reads (§3.3). `transcript` is the file its id
/// names, and a reader reads exactly that file, never "the latest in this
/// place"; once ended, only its first `ended_offset` bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub harness: String,
    pub session: String,
    pub transcript: PathBuf,
    pub bound_at: i64,
    pub state: BindingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
}

impl Binding {
    fn new(live: &Live, now: i64) -> Binding {
        Binding {
            harness: live.harness.clone(),
            session: live.id.clone(),
            transcript: live.transcript.clone(),
            bound_at: now,
            state: BindingState::Active,
            ended_offset: None,
            ended_at: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.state == BindingState::Active
    }

    /// End it now: the transcript's length is where every reader stops.
    fn end(&mut self, now: i64) {
        self.state = BindingState::Ended;
        self.ended_offset = Some(fs::metadata(&self.transcript).map(|m| m.len()).unwrap_or(0));
        self.ended_at = Some(now);
    }

    /// What a guest is shown of it: no path, no id.
    fn guest_view(&self) -> Value {
        json!({ "harness": self.harness, "state": self.state, "bound_at": self.bound_at, "ended_at": self.ended_at })
    }
}

/// A guest's request for a share (§2.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub principal: PrincipalId,
    /// What the guest named: `guest_places`' project id.
    pub project_id: String,
    /// The registry key it resolved to, `None` when it named nothing.
    pub project: Option<String>,
    pub slug: String,
    pub note: Option<String>,
    pub at: i64,
    /// A hidden project's place or one that does not exist: kept so the
    /// answer and the limits behave exactly as for a real one, never listed.
    #[serde(default)]
    pub silent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismissed_at: Option<i64>,
}

impl Request {
    fn open(&self, now: i64) -> bool {
        self.dismissed_at.is_none() && now - self.at < REQUEST_OPEN_FOR
    }
}

// ── the live world a reconcile reads ─────────────────────────────────────────

/// A live session, as the share check sees one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    pub harness: String,
    pub id: String,
    pub transcript: PathBuf,
}

/// What a share's lane is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lane {
    /// The project is not registered, its place directory is gone, or the
    /// directory is another one: the share goes.
    Gone,
    /// No live agent, or one whose id is not readable yet: nothing changes.
    Paused,
    /// A live agent: `Some` with its session, `None` for a harness with no
    /// reader (it is still not the bound session).
    Live(Option<Live>),
}

/// What [`Shares::reconcile`] changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// The share's binding ended: another live session in the lane.
    Ended { share: String, offset: u64 },
    /// The share's lane is gone, and the share with it.
    Removed { share: String },
    /// Requests that lapsed or named a place that went were dropped.
    Pruned(usize),
}

// ── the store ────────────────────────────────────────────────────────────────

/// `shares.json` in one config dir. Holds nothing in memory: every call reads
/// the file, so a request is always judged against the file as it stands.
pub struct Shares {
    dir: PathBuf,
}

/// A fresh id: `prefix` and 80 random bits in lowercase base32.
fn new_id(prefix: &str) -> String {
    let mut b = [0u8; 10];
    getrandom::fill(&mut b).expect("the OS refused entropy");
    format!("{prefix}{}", worktrees_core::pairuri::base32(&b))
}

/// What a guest calls a project: 16 hex of its registry key's SHA-256, so a
/// guest names a project without learning where it lives on the host.
pub fn project_id(root: &str) -> String {
    Sha256::digest(root.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// A place's directory: the project root for `(main)`, else under
/// `.worktrees/` — the resolver's own rule (`resolve::St::slug`).
pub fn place_dir(root: &str, slug: &str) -> PathBuf {
    if slug == "(main)" {
        PathBuf::from(root)
    } else {
        Path::new(root).join(".worktrees").join(slug)
    }
}

fn unreadable(why: &str) -> String {
    format!("{why} — every guest is refused, and the file is left as it is until it is fixed or removed")
}

impl Shares {
    pub fn at(config_dir: &Path) -> Shares {
        Shares { dir: config_dir.to_path_buf() }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE)
    }

    /// The file as it stands. Missing is empty; anything that does not parse
    /// whole, or is not a regular file, is an `Err` that refuses every guest.
    pub fn read(&self) -> Result<SharesFile, String> {
        let path = self.path();
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SharesFile::default()),
            Err(e) => return Err(unreadable(&format!("{}: {e}", path.display()))),
            Ok(m) if !m.file_type().is_file() => return Err(unreadable(&format!("{} is not a regular file", path.display()))),
            Ok(_) => {}
        }
        let bytes = fs::read(&path).map_err(|e| unreadable(&format!("{}: {e}", path.display())))?;
        let f: SharesFile = serde_json::from_slice(&bytes).map_err(|e| unreadable(&format!("{FILE} does not parse: {e}")))?;
        if f.version != VERSION {
            return Err(unreadable(&format!("{FILE} is version {}; this build reads version {VERSION}", f.version)));
        }
        Ok(f)
    }

    /// One change under the lock, against the file as it stands; written,
    /// `0600`, only if it changed. An unreadable file refuses every change.
    pub fn mutate<T>(&self, f: impl FnOnce(&mut SharesFile) -> Result<T, String>) -> Result<T, String> {
        servefile::with_lock(&self.dir, FILE, || {
            let cur = self.read()?;
            let mut next = cur.clone();
            let t = f(&mut next)?;
            if next != cur {
                use std::os::unix::fs::PermissionsExt;
                let mut bytes = serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?;
                bytes.push(b'\n');
                worktrees_core::fsx::write_atomic(&self.path(), &bytes, Some(fs::Permissions::from_mode(0o600)))
                    .map_err(|e| format!("{}: {e}", self.path().display()))?;
            }
            Ok(t)
        })
    }

    // ── the owner's acts ─────────────────────────────────────────────────────

    /// Share place `slug` of `project` with `guests`, bound to `live` (§1
    /// step 2). One share per lane: a lane already shared is answered with
    /// its id, to add people (`share_allow`) or bind anew (`share_again`).
    pub fn create(&self, project: &str, slug: &str, place: PlaceId, guests: &[PrincipalId], live: &Live, now: i64) -> Result<Share, String> {
        shareable(&live.harness)?;
        if guests.is_empty() {
            return Err("a share names at least one guest".into());
        }
        self.mutate(|f| {
            if let Some(s) = f.shares.iter().find(|s| s.project == project && s.slug == slug && s.place == place) {
                return Err(format!("this place is shared already ({}): add a guest with share_allow, or bind the live session with share_again", s.id));
            }
            let mut id = new_id("s_");
            while f.shares.iter().any(|s| s.id == id) {
                id = new_id("s_");
            }
            let mut seen = BTreeSet::new();
            let guests = guests
                .iter()
                .filter(|g| seen.insert(g.as_str().to_string()))
                .map(|g| ShareGuest { principal: g.clone(), state: Admission::Invited, since: now })
                .collect();
            let s = Share {
                id,
                project: project.to_string(),
                slug: slug.to_string(),
                place,
                created_at: now,
                guests,
                binding: Some(Binding::new(live, now)),
                ended: Vec::new(),
            };
            f.shares.push(s.clone());
            Ok(s)
        })
    }

    /// *Share again* (§3.3): bind the lane's CURRENT live session, the ended
    /// binding kept beside it. A binding still active on another id is ended
    /// first, exactly as the poll would have; one active on this very id is
    /// refused, since there is nothing new to bind.
    pub fn share_again(&self, id: &str, live: &Live, now: i64) -> Result<Share, String> {
        shareable(&live.harness)?;
        self.mutate(|f| {
            let s = find_mut(f, id)?;
            if let Some(b) = &mut s.binding {
                if b.is_active() {
                    if b.session == live.id && b.harness == live.harness {
                        return Err("this share is already bound to the live session".into());
                    }
                    b.end(now);
                }
            }
            if let Some(old) = s.binding.take() {
                s.ended.push(old);
            }
            s.binding = Some(Binding::new(live, now));
            Ok(s.clone())
        })
    }

    /// Let `principal` in (§1 step 4), naming them on the share if they were
    /// not. `Ok(false)` when they were allowed already.
    pub fn allow(&self, id: &str, principal: &PrincipalId, now: i64) -> Result<bool, String> {
        self.mutate(|f| {
            let s = find_mut(f, id)?;
            match s.guests.iter_mut().find(|g| &g.principal == principal) {
                Some(g) if g.state == Admission::Allowed => Ok(false),
                Some(g) => {
                    g.state = Admission::Allowed;
                    g.since = now;
                    Ok(true)
                }
                None => {
                    s.guests.push(ShareGuest { principal: principal.clone(), state: Admission::Allowed, since: now });
                    Ok(true)
                }
            }
        })
    }

    /// Take `principal` off the share. Its access ends with this write: every
    /// guest request reads the file. `Ok(false)` if they were not on it.
    pub fn remove_guest(&self, id: &str, principal: &PrincipalId) -> Result<bool, String> {
        self.mutate(|f| {
            let s = find_mut(f, id)?;
            let before = s.guests.len();
            s.guests.retain(|g| &g.principal != principal);
            Ok(s.guests.len() != before)
        })
    }

    /// Take `principal` off EVERY share and drop its open requests (the
    /// console's Remove guest). How many shares named them.
    pub fn remove_principal(&self, principal: &PrincipalId) -> Result<usize, String> {
        self.mutate(|f| {
            let mut n = 0;
            for s in &mut f.shares {
                let before = s.guests.len();
                s.guests.retain(|g| &g.principal != principal);
                n += usize::from(s.guests.len() != before);
            }
            f.requests.retain(|r| &r.principal != principal);
            Ok(n)
        })
    }

    /// Stop sharing: the share and its bindings go. `Ok(false)`: no such share.
    pub fn stop(&self, id: &str) -> Result<bool, String> {
        self.mutate(|f| {
            let before = f.shares.len();
            f.shares.retain(|s| s.id != id);
            Ok(f.shares.len() != before)
        })
    }

    /// The *Show place names to guests* switch, per project (§2.2).
    pub fn hide(&self, project: &str, hidden: bool) -> Result<bool, String> {
        self.mutate(|f| Ok(if hidden { f.hidden_projects.insert(project.to_string()) } else { f.hidden_projects.remove(project) }))
    }

    /// Dismiss a request. `Ok(false)`: no such open request.
    pub fn dismiss(&self, id: &str, now: i64) -> Result<bool, String> {
        self.mutate(|f| match f.requests.iter_mut().find(|r| r.id == id && !r.silent && r.dismissed_at.is_none()) {
            Some(r) => {
                r.dismissed_at = Some(now);
                Ok(true)
            }
            None => Ok(false),
        })
    }

    // ── a guest's acts ───────────────────────────────────────────────────────

    /// A guest asks to join a share it is named on (§1 step 4): grants
    /// nothing. Its standing afterwards.
    pub fn join(&self, id: &str, principal: &PrincipalId, now: i64) -> Result<Admission, String> {
        self.mutate(|f| {
            let s = find_mut(f, id)?;
            let g = s.guests.iter_mut().find(|g| &g.principal == principal).ok_or_else(|| format!("no share {id}"))?;
            if g.state == Admission::Invited {
                g.state = Admission::Asked;
                g.since = now;
            }
            Ok(g.state)
        })
    }

    /// A guest's `share_request` (§2.2). `resolve` maps the guest's project
    /// id to a registry key among the registered projects. The note is
    /// checked FIRST and the rate next, so neither refusal depends on the
    /// place; past them the answer is the same whatever the place is, and a
    /// repeat while one is open is the same answer again.
    pub fn request(
        &self,
        principal: &PrincipalId,
        project_id: &str,
        slug: &str,
        note: Option<&str>,
        now: i64,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        if let Some(n) = note {
            if n.len() > NOTE_MAX {
                return Err(format!("a note is at most {NOTE_MAX} bytes; this one is {}", n.len()));
            }
            worktrees_core::harness::typed_line_ok(n)?;
        }
        if super::resolve::slug_rule(slug).is_err() || project_id.len() > 64 {
            return Err("invalid args: a place is a project id and a slug".into());
        }
        let project = resolve(project_id);
        self.mutate(|f| {
            let recent = f.requests.iter().filter(|r| &r.principal == principal && now - r.at < REQUEST_WINDOW).count();
            if recent >= REQUESTS_PER_WINDOW {
                return Err(format!("rate: at most {REQUESTS_PER_WINDOW} share requests an hour"));
            }
            if f.requests.iter().any(|r| &r.principal == principal && r.project_id == project_id && r.slug == slug && r.open(now)) {
                return Ok(());
            }
            let visible = project.as_ref().filter(|p| !f.hidden_projects.contains(*p) && place_dir(p, slug).is_dir());
            let mut id = new_id("r_");
            while f.requests.iter().any(|r| r.id == id) {
                id = new_id("r_");
            }
            f.requests.push(Request {
                id,
                principal: principal.clone(),
                project_id: project_id.to_string(),
                project: visible.cloned(),
                slug: slug.to_string(),
                note: note.map(str::to_string),
                at: now,
                silent: visible.is_none(),
                dismissed_at: None,
            });
            Ok(())
        })
    }

    // ── the poll ─────────────────────────────────────────────────────────────

    /// §3.3, on serve's start and its poll: each share's lane, as `check`
    /// reads it, decides. Gone removes the share. A live session other than
    /// an ACTIVE binding's ends that binding for good, at the transcript's
    /// length now. Paused changes nothing, and an ended binding is never
    /// re-bound by its id coming back. Requests that lapsed (or never could
    /// be answered, past their window) are dropped too.
    ///
    /// `check` runs OUTSIDE the lock (it reads tmux and the harness files);
    /// the write applies only to shares whose binding is still the one
    /// checked, so an owner's act in between is never undone.
    pub fn reconcile(&self, now: i64, check: &dyn Fn(&Share) -> Lane) -> Result<Vec<Change>, String> {
        let cur = self.read()?;
        let verdicts: Vec<(String, Option<Binding>, Lane)> =
            cur.shares.iter().map(|s| (s.id.clone(), s.binding.clone(), check(s))).collect();
        self.mutate(|f| {
            let mut out = Vec::new();
            for (id, was, lane) in verdicts {
                let Some(pos) = f.shares.iter().position(|s| s.id == id) else { continue };
                match lane {
                    Lane::Gone => {
                        f.shares.remove(pos);
                        out.push(Change::Removed { share: id });
                    }
                    Lane::Paused => {}
                    Lane::Live(live) => {
                        let s = &mut f.shares[pos];
                        if s.binding != was {
                            continue;
                        }
                        let Some(b) = s.binding.as_mut().filter(|b| b.is_active()) else { continue };
                        let same = live.as_ref().is_some_and(|l| l.id == b.session && l.harness == b.harness);
                        if !same {
                            b.end(now);
                            out.push(Change::Ended { share: id, offset: b.ended_offset.unwrap_or(0) });
                        }
                    }
                }
            }
            let before = f.requests.len();
            f.requests.retain(|r| now - r.at < REQUEST_OPEN_FOR.max(REQUEST_WINDOW));
            if f.requests.len() != before {
                out.push(Change::Pruned(before - f.requests.len()));
            }
            Ok(out)
        })
    }

    // ── views ────────────────────────────────────────────────────────────────

    /// The shares `principal` is named on, ended bindings included (§2's
    /// `shares_list`), with what a guest may know of each: the project's
    /// name, the slug and title, its standing, and each binding's harness,
    /// state and times. Never a path, a session id or another guest.
    pub fn guest_shares(&self, principal: &PrincipalId, title: &dyn Fn(&str, &str) -> Option<String>) -> Result<Vec<Value>, String> {
        let f = self.read()?;
        Ok(f.shares
            .iter()
            .filter_map(|s| {
                let g = s.guests.iter().find(|g| &g.principal == principal)?;
                Some(json!({
                    "id": s.id,
                    "project": project_name(&s.project),
                    "project_id": project_id(&s.project),
                    "slug": s.slug,
                    "title": title(&s.project, &s.slug),
                    "state": g.state,
                    "binding": s.binding.as_ref().map(Binding::guest_view),
                    "ended": s.ended.iter().map(Binding::guest_view).collect::<Vec<_>>(),
                    "created_at": s.created_at,
                }))
            })
            .collect())
    }

    /// The share `id` names, for a guest: `Some` only when `principal` is on
    /// it — another guest's share is answered as a missing one.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn share_for(&self, id: &str, principal: &PrincipalId) -> Result<Option<Share>, String> {
        Ok(self.read()?.shares.into_iter().find(|s| s.id == id && s.guests.iter().any(|g| &g.principal == principal)))
    }

    /// The open requests the owner sees (§2.2): real ones only, newest first.
    pub fn open_requests(&self, now: i64) -> Result<Vec<Request>, String> {
        let mut v: Vec<Request> = self.read()?.requests.into_iter().filter(|r| !r.silent && r.open(now)).collect();
        v.sort_by(|a, b| b.at.cmp(&a.at));
        Ok(v)
    }
}

/// Kimi and pi have no transcript reader yet (§3.2, D11): a share of one
/// could show nothing, so it is refused, and says why.
pub fn shareable(harness: &str) -> Result<(), String> {
    if SHAREABLE.contains(&harness) {
        return Ok(());
    }
    Err(format!(
        "{harness} sessions cannot be shared yet: only Claude and Codex have a transcript reader, and a guest of this one would see nothing"
    ))
}

fn find_mut<'a>(f: &'a mut SharesFile, id: &str) -> Result<&'a mut Share, String> {
    f.shares.iter_mut().find(|s| s.id == id).ok_or_else(|| format!("no share {id}"))
}

/// A project's name, as a guest sees it: its folder's.
pub fn project_name(root: &str) -> String {
    Path::new(root).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `guest_places` (§2.2): for every registered project not hidden, its name,
/// its guest id, and each place's slug and title. Nothing else: no note,
/// lifecycle, branch, agent or path. Read from the directory and the
/// declared store, so a guest's read runs no git.
pub fn guest_places(roots: &[String], hidden: &BTreeSet<String>) -> Vec<Value> {
    roots
        .iter()
        .filter(|r| !hidden.contains(*r))
        .map(|root| {
            let declared = worktrees_core::store::read_lenient(root);
            let title = |slug: &str| declared.places.get(slug).and_then(|d| d.title.clone());
            let mut slugs: Vec<String> = vec!["(main)".to_string()];
            if let Ok(rd) = fs::read_dir(Path::new(root).join(".worktrees")) {
                let mut more: Vec<String> = rd
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|s| super::resolve::slug_rule(s).is_ok())
                    .collect();
                more.sort();
                slugs.extend(more);
            }
            json!({
                "id": project_id(root),
                "name": project_name(root),
                "places": slugs.iter().map(|s| json!({ "slug": s, "title": title(s) })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

// ── the live check ───────────────────────────────────────────────────────────

/// Test builds only: what a place's agent "is" — by place directory, so
/// parallel tests over their own scratch roots never meet. Unlisted places
/// ask the real tmux and harness files, which a test never has.
#[cfg(test)]
pub static TEST_LIVE: std::sync::Mutex<std::collections::BTreeMap<PathBuf, worktrees_api::agents::LiveSession>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

fn live_session(root: &str, slug: &str) -> worktrees_api::agents::LiveSession {
    #[cfg(test)]
    if let Some(l) = TEST_LIVE.lock().unwrap().get(&place_dir(root, slug)) {
        return l.clone();
    }
    worktrees_api::agents::place_live_session(root, slug)
}

/// The session `share` or `share_again` binds: the place's live Claude or
/// Codex, or why there is none to bind.
pub fn bindable(root: &str, slug: &str) -> Result<Live, String> {
    use worktrees_api::agents::LiveSession;
    match live_session(root, slug) {
        LiveSession::Running { harness, id, transcript } => Ok(Live { harness: harness.to_string(), id, transcript }),
        LiveSession::Unsupported(label) => shareable(label).map(|()| unreachable!("no reader is unsupported")),
        LiveSession::None => Err("no agent session is running in this place, or it has not named its session yet: start it, then share".into()),
    }
}

/// A share's lane now (§3.3), as [`Shares::reconcile`] judges it. `roots`
/// are every registered project's registry keys.
pub fn lane_now(roots: &[String], s: &Share) -> Lane {
    use worktrees_api::agents::LiveSession;
    if !roots.contains(&s.project) || PlaceId::of(&place_dir(&s.project, &s.slug)) != Some(s.place) {
        return Lane::Gone;
    }
    match live_session(&s.project, &s.slug) {
        LiveSession::None => Lane::Paused,
        LiveSession::Running { harness, id, transcript } => Lane::Live(Some(Live { harness: harness.to_string(), id, transcript })),
        LiveSession::Unsupported(_) => Lane::Live(None),
    }
}

/// One pass of the share check: every share's lane, judged against the
/// registry as it stands. Quiet when there is no `shares.json`. Its changes
/// go to serve.log, by share id only.
pub fn tick(config_dir: &Path) {
    let store = Shares::at(config_dir);
    if !store.path().exists() {
        return;
    }
    let roots = worktrees_core::registry::read_lenient().roots();
    match store.reconcile(worktrees_core::sysclock::now_epoch(), &|s| lane_now(&roots, s)) {
        Ok(changes) => {
            for c in changes {
                worktrees_api::log::info(&format!("serve: shares: {c:?}"));
            }
        }
        Err(e) => worktrees_api::log::warn(&format!("serve: shares: {e}")),
    }
}

/// How often serve checks its shares' bindings and lanes (§3.3: "on its own
/// poll"), whether or not a guest is reading.
pub const TICK: std::time::Duration = std::time::Duration::from_secs(5);

/// The check at start and then every [`TICK`], until serve stops.
pub async fn watch(config_dir: PathBuf) {
    loop {
        let d = config_dir.clone();
        let _ = tokio::task::spawn_blocking(move || tick(&d)).await;
        tokio::time::sleep(TICK).await;
    }
}

// ── the rows ─────────────────────────────────────────────────────────────────

/// The bodies of the share rows (`dispatch.rs`), each given the arguments
/// its row read, after the resolver checked the objects it declares.
pub mod rows {
    use super::*;
    use crate::serve::dispatch::Call;

    fn store(cx: &Call) -> Shares {
        Shares::at(&cx.ctx.config_dir)
    }

    fn now() -> i64 {
        worktrees_core::sysclock::now_epoch()
    }

    /// Every registered root, whatever the caller's scope: a guest's scope is
    /// empty by definition, and `guest_places` is about the HOST's projects.
    fn all_roots(cx: &Call) -> Vec<String> {
        match &cx.world {
            Some(w) => w.roots(),
            None => worktrees_core::registry::read_lenient().roots(),
        }
    }

    fn title(root: &str, slug: &str) -> Option<String> {
        worktrees_core::store::read_lenient(root).places.get(slug).and_then(|d| d.title.clone())
    }

    fn guest_of(cx: &Call, id: &str) -> Result<PrincipalId, String> {
        let p = PrincipalId::new(id);
        cx.devices.guest(&p).map(|_| p).ok_or_else(|| format!("no guest {id}"))
    }

    fn owner_name(cx: &Call) -> Option<String> {
        cx.devices.principal_name(&PrincipalId::owner())
    }

    // The console's.

    pub fn create(cx: &Call, repo: String, slug: String, guests: Vec<String>) -> Result<Value, String> {
        let guests = guests.iter().map(|g| guest_of(cx, g)).collect::<Result<Vec<_>, _>>()?;
        let place = PlaceId::of(&place_dir(&repo, &slug)).ok_or_else(|| format!("no such place: {slug}"))?;
        let live = bindable(&repo, &slug)?;
        serde_json::to_value(store(cx).create(&repo, &slug, place, &guests, &live, now())?).map_err(|e| e.to_string())
    }

    pub fn again(cx: &Call, share: String) -> Result<Value, String> {
        let s = store(cx).read()?.shares.into_iter().find(|s| s.id == share).ok_or_else(|| format!("no share {share}"))?;
        let live = bindable(&s.project, &s.slug)?;
        serde_json::to_value(store(cx).share_again(&share, &live, now())?).map_err(|e| e.to_string())
    }

    pub fn allow(cx: &Call, share: String, principal: String) -> Result<bool, String> {
        let p = guest_of(cx, &principal)?;
        store(cx).allow(&share, &p, now())
    }

    pub fn remove_guest(cx: &Call, share: String, principal: String) -> Result<bool, String> {
        store(cx).remove_guest(&share, &PrincipalId::new(principal))
    }

    pub fn stop(cx: &Call, share: String) -> Result<bool, String> {
        store(cx).stop(&share)
    }

    pub fn hide(cx: &Call, project: String, hidden: bool) -> Result<bool, String> {
        store(cx).hide(&project, hidden)
    }

    pub fn all(cx: &Call) -> Result<Value, String> {
        let f = store(cx).read()?;
        Ok(json!({ "shares": f.shares, "hidden_projects": f.hidden_projects }))
    }

    pub fn requests(cx: &Call) -> Result<Value, String> {
        let v: Vec<Value> = store(cx)
            .open_requests(now())?
            .into_iter()
            .map(|r| {
                json!({
                    "id": r.id, "principal": r.principal, "name": cx.devices.principal_name(&r.principal),
                    "project": r.project, "slug": r.slug, "note": r.note, "at": r.at,
                })
            })
            .collect();
        Ok(Value::from(v))
    }

    pub fn dismiss(cx: &Call, request: String) -> Result<bool, String> {
        store(cx).dismiss(&request, now())
    }

    pub fn guests(cx: &Call) -> Result<Value, String> {
        serde_json::to_value(cx.devices.guests()?).map_err(|e| e.to_string())
    }

    /// Remove a guest from this host: every device revoked (each socket
    /// closes at once), every share and open request of theirs gone. The
    /// person's record stays, so what they wrote is still named.
    pub fn remove_principal(cx: &Call, principal: String) -> Result<Value, String> {
        let p = guest_of(cx, &principal)?;
        let revoked = cx.devices.revoke_guest(&p, now())?;
        let shares = store(cx).remove_principal(&p)?;
        Ok(json!({ "revoked": revoked, "shares": shares }))
    }

    // A guest's.

    pub fn list(cx: &Call) -> Result<Value, String> {
        let shares = store(cx).guest_shares(cx.caller.principal(), &title)?;
        Ok(json!({ "owner": owner_name(cx), "shares": shares }))
    }

    pub fn join(cx: &Call, share: String) -> Result<Value, String> {
        let state = store(cx).join(&share, cx.caller.principal(), now())?;
        Ok(json!({ "state": state }))
    }

    pub fn places(cx: &Call) -> Result<Value, String> {
        let hidden = store(cx).read()?.hidden_projects;
        Ok(json!({ "owner": owner_name(cx), "projects": guest_places(&all_roots(cx), &hidden) }))
    }

    /// Always answered `null` when it is not refused: the same answer for a
    /// real place, a hidden one and one that does not exist (§2.2).
    pub fn request(cx: &Call, project: String, slug: String, note: Option<String>) -> Result<(), String> {
        let roots = all_roots(cx);
        let resolve = |id: &str| roots.iter().find(|r| project_id(r) == id).cloned();
        store(cx).request(cx.caller.principal(), &project, &slug, note.as_deref(), now(), &resolve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const NOW: i64 = 1_800_000_000;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-shares-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::canonicalize(&d).unwrap()
    }

    fn bret() -> PrincipalId {
        PrincipalId::new("g_bret")
    }

    /// A lane: `<top>/repo/.worktrees/feat`, and a transcript of `bytes`.
    struct Lab {
        top: PathBuf,
        shares: Shares,
        root: String,
        place: PlaceId,
    }

    impl Lab {
        fn new(tag: &str) -> Lab {
            let top = dir(tag);
            let root = top.join("repo");
            fs::create_dir_all(root.join(".worktrees/feat")).unwrap();
            let place = PlaceId::of(&root.join(".worktrees/feat")).unwrap();
            Lab { shares: Shares::at(&top.join("config")), root: root.to_string_lossy().into_owned(), top, place }
        }

        fn session(&self, id: &str, bytes: usize) -> Live {
            let t = self.top.join(format!("{id}.jsonl"));
            fs::write(&t, vec![b'x'; bytes]).unwrap();
            Live { harness: "claude".into(), id: id.into(), transcript: t }
        }

        fn grow(&self, live: &Live, more: usize) {
            use std::io::Write;
            fs::OpenOptions::new().append(true).open(&live.transcript).unwrap().write_all(&vec![b'y'; more]).unwrap();
        }

        fn share(&self, live: &Live) -> Share {
            self.shares.create(&self.root, "feat", self.place, &[bret()], live, NOW).unwrap()
        }

        fn only(&self) -> Share {
            let f = self.shares.read().unwrap();
            assert_eq!(f.shares.len(), 1, "{f:#?}");
            f.shares[0].clone()
        }
    }

    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.top);
        }
    }

    #[test]
    fn the_file_is_0600_and_a_missing_one_is_empty() {
        let lab = Lab::new("mode");
        assert_eq!(lab.shares.read().unwrap(), SharesFile::default());
        assert!(!lab.shares.path().exists(), "reading creates nothing");
        lab.share(&lab.session("a", 10));
        let mode = fs::metadata(lab.shares.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "shares.json is {mode:o}");
    }

    /// An unreadable file refuses everything and is never rewritten.
    #[test]
    fn an_unreadable_file_refuses_every_read_and_write_and_is_left_alone() {
        let lab = Lab::new("corrupt");
        lab.share(&lab.session("a", 10));
        let good = fs::read_to_string(lab.shares.path()).unwrap();
        for bad in ["{\"version\": 1, ", &good.replace("\"version\": 1", "\"version\": 2"), &good.replacen('{', "{\"extra\": 1, ", 1)] {
            fs::write(lab.shares.path(), bad).unwrap();
            assert!(lab.shares.read().unwrap_err().contains("every guest is refused"));
            assert!(lab.shares.guest_shares(&bret(), &|_, _| None).is_err());
            assert!(lab.shares.hide(&lab.root, true).is_err());
            assert!(lab.shares.reconcile(NOW, &|_| Lane::Gone).is_err());
            assert_eq!(fs::read_to_string(lab.shares.path()).unwrap(), bad, "left byte for byte");
        }
    }

    /// §3.3: a different live id ends the binding for good, at the
    /// transcript's length; the old id coming back and growing changes
    /// nothing; only Share again binds the new one, and the ended binding
    /// stays readable beside it.
    #[test]
    fn a_new_live_id_ends_the_binding_for_good_frozen_at_its_offset() {
        let lab = Lab::new("end");
        let old = lab.session("old", 100);
        let s = lab.share(&old);
        // The same id, again and again: nothing changes.
        assert_eq!(lab.shares.reconcile(NOW + 1, &|_| Lane::Live(Some(old.clone()))).unwrap(), vec![]);
        lab.grow(&old, 20);
        // No live agent: a pause.
        assert_eq!(lab.shares.reconcile(NOW + 2, &|_| Lane::Paused).unwrap(), vec![]);
        assert!(lab.only().binding.unwrap().is_active());
        // `/clear`: a new id. The binding ends at 120 bytes.
        let new = lab.session("new", 5);
        let ch = lab.shares.reconcile(NOW + 3, &|_| Lane::Live(Some(new.clone()))).unwrap();
        assert_eq!(ch, vec![Change::Ended { share: s.id.clone(), offset: 120 }]);
        let b = lab.only().binding.unwrap();
        assert_eq!((b.state, b.ended_offset, b.ended_at, b.session.as_str()), (BindingState::Ended, Some(120), Some(NOW + 3), "old"));
        // The old id resumed, and grows: still ended, still 120.
        lab.grow(&old, 500);
        assert_eq!(lab.shares.reconcile(NOW + 4, &|_| Lane::Live(Some(old.clone()))).unwrap(), vec![]);
        let b = lab.only().binding.unwrap();
        assert_eq!((b.state, b.ended_offset), (BindingState::Ended, Some(120)), "an ended binding never re-binds");
        // Share again binds the live session; the ended one is kept.
        let again = lab.shares.share_again(&s.id, &new, NOW + 5).unwrap();
        assert_eq!(again.binding.as_ref().map(|b| (b.session.as_str(), b.state)), Some(("new", BindingState::Active)));
        assert_eq!(again.ended.len(), 1);
        assert_eq!((again.ended[0].session.as_str(), again.ended[0].ended_offset), ("old", Some(120)));
        assert!(lab.shares.share_again(&s.id, &new, NOW + 6).unwrap_err().contains("already bound"));
        // A harness with no reader live in the lane is still not the session.
        assert_eq!(lab.shares.reconcile(NOW + 7, &|_| Lane::Live(None)).unwrap().len(), 1);
    }

    /// Share again over a binding the poll has not ended yet ends it first.
    #[test]
    fn share_again_ends_a_stale_active_binding_first() {
        let lab = Lab::new("again");
        let old = lab.session("old", 7);
        let s = lab.share(&old);
        let new = lab.session("new", 1);
        let again = lab.shares.share_again(&s.id, &new, NOW + 1).unwrap();
        assert_eq!(again.ended[0].ended_offset, Some(7));
        assert_eq!(again.ended[0].state, BindingState::Ended);
    }

    /// §3.3: a lane that is gone takes its shares with it, and a later place
    /// with the same slug is another directory and inherits nothing.
    #[test]
    fn a_share_goes_with_its_lane_and_a_new_place_with_the_slug_inherits_nothing() {
        let lab = Lab::new("lane");
        let s = lab.share(&lab.session("a", 1));
        let feat = Path::new(&lab.root).join(".worktrees/feat");
        // The place removed and made again under the same slug (serve down).
        remake(&feat);
        assert!(s.place.born_ns.is_some(), "this filesystem records no birth time: {:?}", s.place);
        assert_ne!(PlaceId::of(&feat), Some(s.place));
        // The live check: what serve's poll hands it.
        let check = |sh: &Share| if PlaceId::of(&place_dir(&sh.project, &sh.slug)) == Some(sh.place) { Lane::Paused } else { Lane::Gone };
        assert_eq!(lab.shares.reconcile(NOW, &check).unwrap(), vec![Change::Removed { share: s.id }]);
        assert!(lab.shares.read().unwrap().shares.is_empty());
    }

    #[test]
    fn kimi_and_pi_cannot_be_shared_and_say_why() {
        let lab = Lab::new("harness");
        for h in ["kimi", "pi"] {
            let mut l = lab.session("k", 1);
            l.harness = h.into();
            let e = lab.shares.create(&lab.root, "feat", lab.place, &[bret()], &l, NOW).unwrap_err();
            assert!(e.contains("cannot be shared yet") && e.contains("transcript reader"), "{e}");
        }
    }

    /// Joining grants nothing; only the owner's allow does.
    #[test]
    fn join_asks_and_allow_admits() {
        let lab = Lab::new("join");
        let s = lab.share(&lab.session("a", 1));
        assert_eq!(lab.shares.join(&s.id, &bret(), NOW).unwrap(), Admission::Asked);
        assert!(lab.shares.join(&s.id, &PrincipalId::new("g_eve"), NOW).unwrap_err().contains("no share"));
        assert!(lab.shares.allow(&s.id, &bret(), NOW).unwrap());
        assert_eq!(lab.shares.join(&s.id, &bret(), NOW).unwrap(), Admission::Allowed);
        assert!(lab.shares.remove_guest(&s.id, &bret()).unwrap());
        assert!(lab.shares.share_for(&s.id, &bret()).unwrap().is_none());
    }

    /// §2.2: a hidden or missing place answers as a real one, a repeat is the
    /// same answer, the rate counts every kind, and the owner sees only real
    /// ones. The note is checked first.
    #[test]
    fn requests_answer_alike_and_only_real_ones_are_listed() {
        let lab = Lab::new("req");
        let pid = project_id(&lab.root);
        let resolve = |id: &str| (id == pid).then(|| lab.root.clone());
        let b = bret();
        assert_eq!(lab.shares.request(&b, &pid, "feat", Some("please"), NOW, &resolve), Ok(()));
        assert_eq!(lab.shares.request(&b, &pid, "feat", Some("again"), NOW, &resolve), Ok(()), "a repeat is the same answer");
        assert_eq!(lab.shares.request(&b, &pid, "nope", None, NOW, &resolve), Ok(()), "a missing place: the same");
        assert_eq!(lab.shares.request(&b, "0000000000000000", "feat", None, NOW, &resolve), Ok(()), "a missing project: the same");
        lab.shares.hide(&lab.root, true).unwrap();
        assert_eq!(lab.shares.request(&b, &pid, "(main)", None, NOW, &resolve), Ok(()), "a hidden project: the same");
        let open = lab.shares.open_requests(NOW).unwrap();
        assert_eq!(open.len(), 1, "{open:#?}");
        assert_eq!((open[0].slug.as_str(), open[0].note.as_deref()), ("feat", Some("please")));
        assert!(lab.shares.request(&b, &pid, "feat", Some("\u{202e}x"), NOW, &resolve).is_err(), "an invisible character");
        assert!(lab.shares.request(&b, &pid, "feat", Some(&"n".repeat(NOTE_MAX + 1)), NOW, &resolve).is_err());
        // The rate: 4 stored so far, all kinds; two more, then refused, for
        // a real place and a fake one alike.
        lab.shares.request(&b, &pid, "x1", None, NOW, &resolve).unwrap();
        lab.shares.request(&b, &pid, "x2", None, NOW, &resolve).unwrap();
        let real = lab.shares.request(&b, &pid, "feat2", None, NOW, &resolve).unwrap_err();
        let fake = lab.shares.request(&b, "ffffffffffffffff", "x3", None, NOW, &resolve).unwrap_err();
        assert_eq!(real, fake);
        assert!(real.starts_with("rate"), "{real}");
        assert!(lab.shares.dismiss(&open[0].id, NOW).unwrap());
        assert!(lab.shares.open_requests(NOW).unwrap().is_empty());
    }

    /// `guest_places`: names only, and nothing of a hidden project.
    #[test]
    fn guest_places_are_names_only_and_hide_a_hidden_project() {
        let lab = Lab::new("names");
        fs::write(Path::new(&lab.root).join(".worktrees.places.json"), r#"{"version":1,"places":{"feat":{"title":"Secret feature","note":"the note","lifecycle":"saved"}}}"#).unwrap();
        let roots = vec![lab.root.clone()];
        let v = guest_places(&roots, &BTreeSet::new());
        assert_eq!(
            v,
            vec![json!({"id": project_id(&lab.root), "name": "repo", "places": [{"slug": "(main)", "title": null}, {"slug": "feat", "title": "Secret feature"}]})]
        );
        assert!(!v[0].to_string().contains(&lab.root), "no path");
        assert!(!v[0].to_string().contains("the note") && !v[0].to_string().contains("saved"));
        assert!(guest_places(&roots, &BTreeSet::from([lab.root.clone()])).is_empty());
    }
}

/// The share rows through the dispatcher, as serve runs them: the device
/// registry is `devices.json` in a scratch config dir, with the owner and two
/// guests, Bret and Eve; objects resolve against a fixture world.
#[cfg(test)]
mod serve_tests {
    use super::*;
    use crate::serve::devices::{DeviceRegistry, Enrollment, Expiry, Platform, PrincipalRecord};
    use crate::serve::dispatch::{self, Args, Call, Exposure, GUEST_ROWS, TABLE};
    use crate::serve::resolve::fixture::{call_as, git_repo, s, scratch, Fixture};
    use crate::serve::resolve::{self as res};
    use std::sync::Arc;
    use worktrees_api::agents::LiveSession;
    use worktrees_core::authz::{self, Caller, Capability, DeviceId, Grant};

    struct Host {
        top: PathBuf,
        cfg: PathBuf,
        a: String,
        b: String,
        devices: Arc<DeviceRegistry>,
        world: Fixture,
        bret: Caller,
        eve: Caller,
    }

    fn guest(devices: &DeviceRegistry, name: &str, key: u8) -> Caller {
        let id = crate::serve::devices::new_guest_id();
        let e = Enrollment {
            principal: id.clone(),
            name: format!("{name}'s MacBook"),
            platform: Platform::Macos,
            spki_sha256: crate::serve::devices::hex(&[key; 32]),
            stepup_spki_sha256: None,
            grant: crate::serve::pairing::Invite::grant(),
            expiry: Expiry::default(),
            addr: None,
        };
        let d = devices.enroll_creating(e, Some(PrincipalRecord::guest(id.clone(), name, NOW)), NOW).unwrap();
        Caller::paired(id, d.id)
    }

    const NOW: i64 = 1_800_000_000;

    impl Host {
        fn new(tag: &str) -> Host {
            let top = scratch(&format!("shares-{tag}"));
            let (a, b) = (top.join("a"), top.join("b"));
            for r in [&a, &b] {
                git_repo(r);
                fs::create_dir_all(r.join(".worktrees/feat")).unwrap();
                fs::create_dir_all(r.join(".worktrees/other")).unwrap();
            }
            let cfg = top.join("config");
            fs::create_dir_all(&cfg).unwrap();
            let devices = Arc::new(DeviceRegistry::open(&cfg));
            let bret = guest(&devices, "Bret", 1);
            let eve = guest(&devices, "Eve", 2);
            let world = Fixture { roots: vec![s(&a), s(&b)], panes: vec![] };
            Host { a: s(&a), b: s(&b), top, cfg, devices, world, bret, eve }
        }

        fn call(&self, who: &Caller) -> Call {
            let base = call_as(who.clone(), self.devices.clone());
            struct Quiet;
            impl worktrees_api::Host for Quiet {
                fn emit(&self, _: &str, _: Value) {}
            }
            Call {
                ctx: worktrees_api::Ctx::new(self.cfg.clone(), Arc::new(Quiet)),
                devices: self.devices.clone(),
                world: Some(Arc::new(Fixture { roots: self.world.roots.clone(), panes: vec![] })),
                ..base
            }
        }

        fn run(&self, who: &Caller, cmd: &str, a: Value) -> Result<Value, Value> {
            dispatch::call_in(&self.call(who), cmd, &Args(a.as_object().cloned().unwrap_or_default()), &self.world)
        }

        fn console(&self, cmd: &str, a: Value) -> Result<Value, Value> {
            self.run(&Caller::console(), cmd, a)
        }

        /// A live Claude in place `slug` of `a`, on session `id`, whose
        /// transcript holds `bytes`.
        fn live(&self, slug: &str, id: &str, bytes: usize) -> PathBuf {
            let t = self.top.join(format!("{id}.jsonl"));
            if !t.exists() {
                fs::write(&t, vec![b'x'; bytes]).unwrap();
            }
            TEST_LIVE
                .lock()
                .unwrap()
                .insert(place_dir(&self.a, slug), LiveSession::Running { harness: "claude", id: id.into(), transcript: t.clone() });
            t
        }

        fn poll(&self) -> Vec<Change> {
            Shares::at(&self.cfg).reconcile(worktrees_core::sysclock::now_epoch(), &|s| lane_now(&self.world.roots, s)).unwrap()
        }

        fn principal(c: &Caller) -> String {
            c.principal().as_str().to_string()
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            let mut l = TEST_LIVE.lock().unwrap();
            l.retain(|k, _| !k.starts_with(&self.top));
            let _ = fs::remove_dir_all(&self.top);
        }
    }

    /// §8: a guest calling ANY row but the guest rows is refused, generated
    /// from the table, so a new row cannot slip through — `own` rows
    /// included, which every other caller may make. Refused at the
    /// exposure-and-capability step, before any object or body.
    #[test]
    fn a_guest_is_refused_on_every_row_but_the_guest_rows() {
        let h = Host::new("drift");
        let grant = authz::effective(&*h.devices, &h.bret);
        assert_eq!(grant, Grant { capabilities: worktrees_core::authz::Caps::GUEST, projects: worktrees_core::authz::Projects::only(Vec::<String>::new()) });
        let mut reached = Vec::new();
        for row in TABLE.iter().filter(|r| !GUEST_ROWS.contains(&r.name)) {
            if res::admissible(row, &h.bret, &grant).is_ok() {
                reached.push(row.name);
            }
            // A refusal, or for a yes/no probe its refusal's answer, `false`.
            let r = h.run(&h.bret, row.name, json!({}));
            let probe = row.objects.iter().any(res::Obj::answers_false);
            assert!(r.is_err() || (probe && r == Ok(Value::Bool(false))), "{} answered a guest: {r:?}", row.name);
        }
        assert!(reached.is_empty(), "rows a guest passes admission on: {reached:?}");
        // GUEST_ROWS is exactly the `guest` rows, and every one of them is an
        // Engine row that names no project, place, file, session or stream.
        let guest_rows: Vec<&str> = TABLE.iter().filter(|r| r.cap == Some(Capability::Guest)).map(|r| r.name).collect();
        assert_eq!(guest_rows, GUEST_ROWS.to_vec());
        for row in TABLE.iter().filter(|r| GUEST_ROWS.contains(&r.name)) {
            assert_eq!(row.exposure, Exposure::Engine, "{}", row.name);
            assert!(row.objects.iter().all(|o| matches!(o, res::Obj::Guest | res::Obj::Share(_))), "{}: {:?}", row.name, row.objects);
            assert!(!dispatch::needs_stepup(row, &[Capability::Guest], false), "{}: guest needs no step-up", row.name);
        }
    }

    /// §8's proof, as Bret on a paired device: the share shared with him is
    /// listed and joinable, Eve's is not his to see or name, nothing is
    /// readable until allowed, and the console decides everything.
    #[test]
    fn bret_sees_what_is_shared_with_him_and_nothing_else() {
        let h = Host::new("bret");
        h.live("feat", "s-1", 10);
        h.live("other", "s-9", 10);
        let bret = Host::principal(&h.bret);
        let eve = Host::principal(&h.eve);
        let share = h.console("share_create", json!({"repo": h.a, "slug": "feat", "guests": [bret]})).unwrap();
        let id = share["id"].as_str().unwrap().to_string();
        let eves = h.console("share_create", json!({"repo": h.a, "slug": "other", "guests": [eve]})).unwrap();
        let eves = eves["id"].as_str().unwrap().to_string();
        // A guest that is not one, or a place with no live session, is refused.
        assert!(h.console("share_create", json!({"repo": h.b, "slug": "feat", "guests": [bret]})).unwrap_err().as_str().unwrap().contains("no agent session"));
        assert!(h.console("share_create", json!({"repo": h.a, "slug": "feat", "guests": ["owner"]})).is_err());
        // Bret's list: his share only, with no path and no session id.
        let list = h.run(&h.bret, "shares_list", json!({})).unwrap();
        let shares = list["shares"].as_array().unwrap();
        assert_eq!(shares.len(), 1, "{list:#}");
        assert_eq!((shares[0]["id"].as_str(), shares[0]["slug"].as_str(), shares[0]["state"].as_str()), (Some(id.as_str()), Some("feat"), Some("invited")));
        assert_eq!(shares[0]["binding"]["state"], json!("active"));
        assert!(!list.to_string().contains(&h.a) && !list.to_string().contains("s-1"), "no path, no session id: {list}");
        // Joining asks; it grants nothing until the console allows.
        assert_eq!(h.run(&h.bret, "share_join", json!({"share": id})).unwrap(), json!({"state": "asked"}));
        assert_eq!(h.run(&h.bret, "share_join", json!({"share": eves})), Err(json!(res::ERR_NOT_FOUND)), "Eve's share reads as missing");
        assert_eq!(h.run(&h.bret, "share_join", json!({"share": "s_nope"})), Err(json!(res::ERR_NOT_FOUND)));
        assert_eq!(h.console("share_allow", json!({"share": id, "principal": bret})).unwrap(), json!(true));
        assert_eq!(h.run(&h.bret, "share_join", json!({"share": id})).unwrap(), json!({"state": "allowed"}));
        // The console's acts are console-only: Bret reaches none of them.
        for (cmd, a) in [("share_allow", json!({"share": eves, "principal": bret})), ("share_stop", json!({"share": id})), ("shares_all", json!({}))] {
            assert!(h.run(&h.bret, cmd, a).is_err(), "{cmd}");
        }
        // Removing Bret from the share ends it for him on the next request.
        assert_eq!(h.console("share_remove_guest", json!({"share": id, "principal": bret})).unwrap(), json!(true));
        assert_eq!(h.run(&h.bret, "shares_list", json!({})).unwrap()["shares"], json!([]));
        assert_eq!(h.run(&h.bret, "share_join", json!({"share": id})), Err(json!(res::ERR_NOT_FOUND)));
        // And Stop sharing removes the share.
        assert_eq!(h.console("share_stop", json!({"share": eves})).unwrap(), json!(true));
        assert_eq!(h.run(&h.eve, "shares_list", json!({})).unwrap()["shares"], json!([]));
    }

    /// §8: no reading of a later session after `/clear` until Share again; an
    /// ended binding is frozen at its offset even after the old id resumes;
    /// the ended one stays listed beside the new.
    #[test]
    fn a_clear_ends_the_binding_and_only_share_again_binds_the_new_session() {
        let h = Host::new("clear");
        let old = h.live("feat", "s-old", 100);
        let bret = Host::principal(&h.bret);
        let id = h.console("share_create", json!({"repo": h.a, "slug": "feat", "guests": [bret]})).unwrap()["id"].as_str().unwrap().to_string();
        assert_eq!(h.poll(), vec![]);
        // Exit: no live agent. A pause.
        TEST_LIVE.lock().unwrap().insert(place_dir(&h.a, "feat"), LiveSession::None);
        assert_eq!(h.poll(), vec![]);
        // `/clear`: a new id. The binding ends at 100 bytes, on the poll.
        h.live("feat", "s-new", 5);
        assert_eq!(h.poll(), vec![Change::Ended { share: id.clone(), offset: 100 }]);
        let f = Shares::at(&h.cfg).read().unwrap();
        let b = f.shares[0].binding.clone().unwrap();
        assert_eq!((b.session.as_str(), b.state, b.ended_offset), ("s-old", BindingState::Ended, Some(100)));
        assert_eq!(h.run(&h.bret, "shares_list", json!({})).unwrap()["shares"][0]["binding"]["state"], json!("ended"));
        // The old id resumed and grows: still ended, still 100.
        h.live("feat", "s-old", 0);
        std::fs::OpenOptions::new().append(true).open(&old).map(|mut f| std::io::Write::write_all(&mut f, b"more")).unwrap().unwrap();
        assert_eq!(h.poll(), vec![]);
        let b = Shares::at(&h.cfg).read().unwrap().shares[0].binding.clone().unwrap();
        assert_eq!((b.state, b.ended_offset), (BindingState::Ended, Some(100)));
        // Share again binds whatever is live now, the ended one kept.
        h.live("feat", "s-new", 5);
        let again = h.console("share_again", json!({"share": id})).unwrap();
        assert_eq!(again["binding"]["session"], json!("s-new"));
        assert_eq!(again["ended"][0]["session"], json!("s-old"));
        assert_eq!(again["ended"][0]["ended_offset"], json!(100));
        let mine = h.run(&h.bret, "shares_list", json!({})).unwrap();
        assert_eq!(mine["shares"][0]["binding"]["state"], json!("active"));
        assert_eq!(mine["shares"][0]["ended"][0]["state"], json!("ended"));
        // Kimi and pi cannot be bound, and say why.
        TEST_LIVE.lock().unwrap().insert(place_dir(&h.a, "other"), LiveSession::Unsupported("Kimi"));
        let e = h.console("share_create", json!({"repo": h.a, "slug": "other", "guests": [bret]})).unwrap_err();
        assert!(e.as_str().unwrap().contains("Kimi sessions cannot be shared yet"), "{e}");
    }

    /// §8: removing the lane with serve up (the poll) or down (the next
    /// start's first pass, which is the same check) removes its shares, and
    /// a new place with the same slug inherits none.
    #[test]
    fn a_removed_lane_takes_its_shares_and_a_new_one_inherits_nothing() {
        let h = Host::new("lane");
        h.live("feat", "s-1", 1);
        h.live("other", "s-2", 1);
        let bret = Host::principal(&h.bret);
        let one = h.console("share_create", json!({"repo": h.a, "slug": "feat", "guests": [bret]})).unwrap()["id"].as_str().unwrap().to_string();
        let two = h.console("share_create", json!({"repo": h.a, "slug": "other", "guests": [bret]})).unwrap()["id"].as_str().unwrap().to_string();
        // "Serve down": the place removed and remade under the same slug
        // before anything looks.
        remake(&place_dir(&h.a, "feat"));
        // "Serve up": the other place simply removed.
        fs::remove_dir_all(place_dir(&h.a, "other")).unwrap();
        let mut ch = h.poll();
        ch.sort_by_key(|c| format!("{c:?}"));
        let mut want = vec![Change::Removed { share: one.clone() }, Change::Removed { share: two }];
        want.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(ch, want);
        assert_eq!(h.run(&h.bret, "shares_list", json!({})).unwrap()["shares"], json!([]));
        assert_eq!(h.run(&h.bret, "share_join", json!({"share": one})), Err(json!(res::ERR_NOT_FOUND)));
        // An unregistered project is a removed lane too.
        h.live("feat", "s-3", 1);
        h.console("share_create", json!({"repo": h.a, "slug": "feat", "guests": [bret]})).unwrap();
        let gone = Shares::at(&h.cfg).reconcile(NOW, &|sh| lane_now(&[h.b.clone()], sh)).unwrap();
        assert_eq!(gone.len(), 1);
    }

    /// §8: `guest_places` lists no hidden project and nothing but names, and
    /// a request for a hidden or missing place answers exactly as a real one.
    #[test]
    fn hidden_projects_vanish_and_requests_answer_alike() {
        let h = Host::new("names");
        let names = h.run(&h.bret, "guest_places", json!({})).unwrap();
        let projects = names["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);
        let slugs: Vec<&str> = projects[0]["places"].as_array().unwrap().iter().map(|p| p["slug"].as_str().unwrap()).collect();
        assert_eq!(slugs, vec!["(main)", "feat", "other"]);
        assert!(!names.to_string().contains(&h.a), "no path: {names}");
        assert_eq!(h.console("guest_places_hide", json!({"project": h.b, "hidden": true})).unwrap(), json!(true));
        let names = h.run(&h.bret, "guest_places", json!({})).unwrap();
        assert_eq!(names["projects"].as_array().unwrap().len(), 1);
        assert_eq!(names["projects"][0]["id"], json!(project_id(&h.a)));
        assert!(!names.to_string().contains(&project_id(&h.b)), "the hidden project is absent, name and id: {names}");
        // Requests: real, hidden, missing place, missing project — one answer.
        let ask = |project: &str, slug: &str| h.run(&h.bret, "share_request", json!({"project": project, "slug": slug, "note": "could I watch?"}));
        let real = ask(&project_id(&h.a), "feat");
        assert_eq!(real, Ok(Value::Null));
        assert_eq!(ask(&project_id(&h.b), "feat"), real, "a hidden project's place");
        assert_eq!(ask(&project_id(&h.a), "nope"), real, "a missing place");
        assert_eq!(ask("0123456789abcdef", "feat"), real, "a missing project");
        assert_eq!(ask(&project_id(&h.a), "feat"), real, "a repeat");
        // The console sees the one real request, by name, and dismisses it.
        let open = h.console("share_requests_list", json!({})).unwrap();
        assert_eq!(open.as_array().unwrap().len(), 1, "{open:#}");
        assert_eq!((open[0]["name"].as_str(), open[0]["slug"].as_str(), open[0]["note"].as_str()), (Some("Bret"), Some("feat"), Some("could I watch?")));
        assert_eq!(h.console("share_request_dismiss", json!({"request": open[0]["id"]})).unwrap(), json!(true));
        assert_eq!(h.console("share_requests_list", json!({})).unwrap(), json!([]));
        // Bret never sees requests, his own or anyone's.
        assert!(h.run(&h.bret, "share_requests_list", json!({})).is_err());
    }

    /// §8: an unreadable `shares.json` refuses every guest, on every guest
    /// row, and is left as it is.
    #[test]
    fn an_unreadable_shares_file_refuses_every_guest() {
        let h = Host::new("unreadable");
        h.live("feat", "s-1", 1);
        let bret = Host::principal(&h.bret);
        let id = h.console("share_create", json!({"repo": h.a, "slug": "feat", "guests": [bret]})).unwrap()["id"].as_str().unwrap().to_string();
        assert!(h.run(&h.bret, "shares_list", json!({})).is_ok());
        fs::write(h.cfg.join(FILE), b"{\"version\": 1, \"shares\": [").unwrap();
        for (cmd, a) in [
            ("shares_list", json!({})),
            ("guest_places", json!({})),
            ("share_join", json!({"share": id})),
            ("share_request", json!({"project": project_id(&h.a), "slug": "feat"})),
        ] {
            assert_eq!(h.run(&h.bret, cmd, a.clone()), Err(json!(res::ERR_DENIED)), "{cmd}");
            assert_eq!(h.run(&h.eve, cmd, a), Err(json!(res::ERR_DENIED)), "{cmd}");
        }
        assert!(h.console("share_stop", json!({"share": id})).is_err(), "nor is it overwritten");
        assert_eq!(fs::read(h.cfg.join(FILE)).unwrap(), b"{\"version\": 1, \"shares\": [");
    }

    /// Removing a guest from the host revokes every device of theirs and
    /// takes them off every share; they are then refused at the door, as any
    /// revoked device is.
    #[test]
    fn removing_a_guest_revokes_their_devices_and_their_shares() {
        let h = Host::new("remove");
        h.live("feat", "s-1", 1);
        let bret = Host::principal(&h.bret);
        h.console("share_create", json!({"repo": h.a, "slug": "feat", "guests": [bret]})).unwrap();
        let guests = h.console("guests_list", json!({})).unwrap();
        assert_eq!(guests.as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["Bret", "Eve"]);
        let out = h.console("guest_remove", json!({"principal": bret})).unwrap();
        assert_eq!((out["revoked"].as_array().unwrap().len(), out["shares"].as_u64()), (1, Some(1)));
        assert_eq!(authz::effective(&*h.devices, &h.bret), Grant::nothing());
        assert!(h.run(&h.bret, "shares_list", json!({})).is_err());
        // The owner gets a display name, which the guest's listing shows.
        assert_eq!(h.console("owner_set_name", json!({"name": "David"})).unwrap(), json!(true));
        assert_eq!(h.run(&h.eve, "shares_list", json!({})).unwrap()["owner"], json!("David"));
        let _ = DeviceId::new("unused");
    }
}
