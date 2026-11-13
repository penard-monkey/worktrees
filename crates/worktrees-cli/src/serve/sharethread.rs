//! The thread beside a share (group chat, `docs/proposals/group-chat.md` §4):
//! one log per share, `<config_dir>/share-threads/<share>.json`, `0600` in a
//! `0700` directory, written whole through `fsx::write_atomic` (which syncs
//! the file before its rename) under `<share>.json.lock`, as `shares.json` is.
//!
//! **Not `worktrees_core::messages`.** That log is agent mail: capped, swept,
//! ids from a clock and a process-local counter, and it is the bus agents
//! READ. This one is inert text between people, and nothing reads it for an
//! agent: no MCP tool, no `wait` wake-up, no hook (`provenance.rs`'s source
//! test holds the MCP server, the hooks and `messages.rs` to that). Only the
//! owner's *Use these* (PR D) puts any of it in front of an agent, through
//! `agent_attach` and `agent_send`, as the console.
//!
//! **Order.** Every entry, a post or an event, takes the next `seq`, and the
//! file holds the next one to give, so a restart keeps the order. The write
//! completes before a post is acknowledged.
//!
//! **Retry.** A post carries a client-made `id`. The same principal posting
//! the same id again is answered with the entry it already made, not a
//! second one: a timed-out retry is safe. The same id with other text is
//! refused rather than taken as either.
//!
//! **Who.** `from` is the caller's principal (and its device), set here from
//! the connection, never from the request. A reader is shown the principal's
//! name and the device's name as the registry has them NOW; the record keeps
//! ids, so a rename never rewrites who said what.
//!
//! **Text** passes `harness::shown_text_ok`: `typed_line_ok`'s character
//! rule, line breaks allowed, refused never stripped, at most
//! [`TEXT_MAX`] (`messages::MAX_TEXT`'s precedent). A leading `/ @ ! #` is
//! allowed, since nothing ever types it.
//!
//! **Lifetime.** A thread goes with its share: *Stop sharing*, and a lane's
//! removal on the poll, delete it. There is no separate delete row: a thread
//! without its share could be read by nobody (`thread_read` resolves the
//! share first).
//!
//! An unreadable thread file refuses reading and posting in THAT thread and
//! is left as it is, as `shares.json` is.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use worktrees_core::authz::{DeviceId, PrincipalId};

use super::servefile;

pub const DIR: &str = "share-threads";

/// The format this build reads and writes; a higher one is refused.
pub const VERSION: u32 = 1;

/// One post, at most, in bytes (§4: `messages::MAX_TEXT`'s precedent).
pub const TEXT_MAX: usize = worktrees_core::messages::MAX_TEXT;

/// A page, at most, in entries.
pub const PAGE_MAX: usize = 200;

/// A thread, at most, in entries: 5000 posts at 8 KiB bound one file at
/// ~40 MB. Past it a post is refused, saying so; Stop sharing clears it.
pub const ENTRIES_MAX: usize = 5000;

/// A client's post id, at most, in bytes: `[A-Za-z0-9_-]`, a UUID fits.
pub const ID_MAX: usize = 64;

// ── the record ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadFile {
    pub version: u32,
    pub share: String,
    /// The seq the next entry takes. Starts at 1.
    pub next: u64,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

/// What an entry is: a post, or one of §4's events, in the same sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Post,
    /// A guest was let in (`share_allow`).
    Joined,
    /// A guest left on their own. No row makes it yet: reserved.
    Left,
    /// The owner took a guest off the share, or off the host.
    Removed,
    /// The owner shared posts `first..=last` with the agent (*Use these*,
    /// PR D). Reserved: no row appends it yet.
    SharedWithAgent,
    /// The binding ended: another session in the lane.
    SessionEnded,
    /// *Share again* bound the lane's live session.
    SharedAgain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub seq: u64,
    pub at: i64,
    pub kind: Kind,
    /// A post's client id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Who: a post's author, or the guest an event is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<PrincipalId>,
    /// A post's device; `None` for the console.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `SharedWithAgent`'s range of posts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<u64>,
}

impl Entry {
    fn event(kind: Kind, principal: Option<&PrincipalId>, at: i64) -> Entry {
        Entry { seq: 0, at, kind, id: None, principal: principal.cloned(), device: None, text: None, first: None, last: None }
    }
}

/// A page of a thread: entries after `after`, in order, and whether more
/// follow. `last` is the newest seq the thread holds (0: none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub entries: Vec<Entry>,
    pub more: bool,
    pub last: u64,
}

// ── the store ────────────────────────────────────────────────────────────────

/// The threads of one config dir. Holds nothing in memory.
pub struct Threads {
    dir: PathBuf,
}

/// A share id names a file here, so it must be a share id's shape and
/// nothing else: `s_` and lowercase base32 (`shares::new_id`).
fn share_file_name(share: &str) -> Result<String, String> {
    let ok = share.len() <= 64 && share.starts_with("s_") && share[2..].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && share.len() > 2;
    if !ok {
        return Err(format!("not a share id: {share}"));
    }
    Ok(format!("{share}.json"))
}

fn post_id_ok(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > ID_MAX || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("invalid args: a post id is 1 to {ID_MAX} of A-Z a-z 0-9 - _"));
    }
    Ok(())
}

fn unreadable(why: &str) -> String {
    format!("{why} — this thread is refused, and its file is left as it is until it is fixed or removed")
}

impl Threads {
    pub fn at(config_dir: &Path) -> Threads {
        Threads { dir: config_dir.join(DIR) }
    }

    pub fn path(&self, share: &str) -> Result<PathBuf, String> {
        Ok(self.dir.join(share_file_name(share)?))
    }

    /// The thread as it stands. Missing is empty.
    pub fn read(&self, share: &str) -> Result<ThreadFile, String> {
        let path = self.path(share)?;
        let empty = ThreadFile { version: VERSION, share: share.to_string(), next: 1, entries: Vec::new() };
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty),
            Err(e) => return Err(unreadable(&format!("{}: {e}", path.display()))),
            Ok(m) if !m.file_type().is_file() => return Err(unreadable(&format!("{} is not a regular file", path.display()))),
            Ok(_) => {}
        }
        let bytes = fs::read(&path).map_err(|e| unreadable(&format!("{}: {e}", path.display())))?;
        let f: ThreadFile = serde_json::from_slice(&bytes).map_err(|e| unreadable(&format!("the thread of {share} does not parse: {e}")))?;
        if f.version != VERSION {
            return Err(unreadable(&format!("the thread of {share} is version {}; this build reads version {VERSION}", f.version)));
        }
        if f.share != share || f.entries.iter().any(|e| e.seq >= f.next) || f.entries.windows(2).any(|w| w[0].seq >= w[1].seq) {
            return Err(unreadable(&format!("the thread of {share} is out of order")));
        }
        Ok(f)
    }

    /// One change under the thread's lock; written, synced, `0600`, only if
    /// it changed. An unreadable file refuses every change.
    fn mutate<T>(&self, share: &str, f: impl FnOnce(&mut ThreadFile) -> Result<T, String>) -> Result<T, String> {
        let name = share_file_name(share)?;
        self.ensure_dir()?;
        servefile::with_lock(&self.dir, &name, || {
            let cur = self.read(share)?;
            let mut next = cur.clone();
            let t = f(&mut next)?;
            if next != cur {
                use std::os::unix::fs::PermissionsExt;
                let mut bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
                bytes.push(b'\n');
                let path = self.dir.join(&name);
                worktrees_core::fsx::write_atomic(&path, &bytes, Some(fs::Permissions::from_mode(0o600))).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            Ok(t)
        })
    }

    fn ensure_dir(&self) -> Result<(), String> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        match fs::symlink_metadata(&self.dir) {
            Ok(m) if m.file_type().is_dir() => Ok(()),
            Ok(_) => Err(format!("{} is not a directory", self.dir.display())),
            Err(_) => {
                if let Some(p) = self.dir.parent() {
                    fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
                }
                match fs::DirBuilder::new().mode(0o700).create(&self.dir) {
                    Ok(()) => fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string()),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                    Err(e) => Err(format!("{}: {e}", self.dir.display())),
                }
            }
        }
    }

    fn push(f: &mut ThreadFile, mut e: Entry) -> Result<Entry, String> {
        if f.entries.len() >= ENTRIES_MAX {
            return Err(format!("this thread is full ({ENTRIES_MAX} entries); stopping the share clears it"));
        }
        e.seq = f.next;
        f.next += 1;
        f.entries.push(e.clone());
        Ok(e)
    }

    /// A post (§4). `Ok((entry, true))` for a new one; `Ok((entry, false))`
    /// for a retry, `from`'s earlier post with this `id` and this text.
    pub fn post(&self, share: &str, id: &str, from: &PrincipalId, device: Option<&DeviceId>, text: &str, now: i64) -> Result<(Entry, bool), String> {
        post_id_ok(id)?;
        let text = worktrees_core::harness::shown_text_ok(text, TEXT_MAX)?;
        self.mutate(share, |f| {
            if let Some(e) = f.entries.iter().find(|e| e.kind == Kind::Post && e.id.as_deref() == Some(id) && e.principal.as_ref() == Some(from)) {
                if e.text.as_deref() != Some(text.as_str()) {
                    return Err(format!("post {id} was already made with other text; a retry sends the same text, a new post a new id"));
                }
                return Ok((e.clone(), false));
            }
            let e = Entry { kind: Kind::Post, id: Some(id.to_string()), device: device.cloned(), text: Some(text), ..Entry::event(Kind::Post, Some(from), now) };
            Ok((Self::push(f, e)?, true))
        })
    }

    /// An event (§4), in the same sequence as the posts.
    pub fn event(&self, share: &str, kind: Kind, principal: Option<&PrincipalId>, now: i64) -> Result<Entry, String> {
        debug_assert!(kind != Kind::Post);
        self.mutate(share, |f| Self::push(f, Entry::event(kind, principal, now)))
    }

    /// `SharedWithAgent` (PR D's *Use these*): posts `first..=last`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn shared_with_agent(&self, share: &str, first: u64, last: u64, now: i64) -> Result<Entry, String> {
        if first > last {
            return Err("first is after last".into());
        }
        self.mutate(share, |f| Self::push(f, Entry { first: Some(first), last: Some(last), ..Entry::event(Kind::SharedWithAgent, None, now) }))
    }

    /// The entries after `after` (0: from the start), at most `limit`
    /// (capped at [`PAGE_MAX`]), in order.
    pub fn page(&self, share: &str, after: u64, limit: usize) -> Result<Page, String> {
        let f = self.read(share)?;
        let limit = limit.clamp(1, PAGE_MAX);
        let mut it = f.entries.into_iter().filter(|e| e.seq > after);
        let entries: Vec<Entry> = it.by_ref().take(limit).collect();
        let more = it.next().is_some();
        Ok(Page { entries, more, last: f.next - 1 })
    }

    /// The thread goes with its share. `Ok(false)`: there was none.
    pub fn remove(&self, share: &str) -> Result<bool, String> {
        let name = share_file_name(share)?;
        let path = self.dir.join(&name);
        let had = match fs::remove_file(&path) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let _ = fs::remove_file(self.dir.join(format!("{name}.lock")));
        Ok(had)
    }
}

// ── `shares:changed` (§6) ────────────────────────────────────────────────────

/// The event's name. Its payload is ids only: `what` happened, the `share`
/// it happened to (absent for a request, whose id a guest never learns),
/// a post's `seq`. Never text or a name. `to` lists the principals whose
/// sockets hear it, besides the console's; `scope::event` routes by it and
/// cuts it from what every socket is sent.
pub const EVENT: &str = "shares:changed";

/// Emit [`EVENT`] for `to` (and the console).
pub fn changed(host: &dyn worktrees_api::Host, what: &str, share: Option<&str>, to: &[PrincipalId], seq: Option<u64>) {
    let mut p = json!({ "what": what, "to": to.iter().map(PrincipalId::as_str).collect::<Vec<_>>() });
    if let Some(s) = share {
        p["share"] = json!(s);
    }
    if let Some(n) = seq {
        p["seq"] = json!(n);
    }
    host.emit(EVENT, p);
}

// ── the rows ─────────────────────────────────────────────────────────────────

pub mod rows {
    use super::*;
    use crate::serve::dispatch::Call;
    use crate::serve::shares::{Admission, Share, Shares};
    use worktrees_core::authz::{Device, Registry};

    fn now() -> i64 {
        worktrees_core::sysclock::now_epoch()
    }

    /// The share, for this caller: the console reads any; a guest only one
    /// that has let it in. The resolver already answered a share that does
    /// not name the guest as missing; one that names it but has not let it
    /// in yet is refused here, saying so.
    fn admitted(cx: &Call, share: &str) -> Result<Share, String> {
        let s = Shares::at(&cx.ctx.config_dir).read()?.shares.into_iter().find(|s| s.id == share).ok_or_else(|| format!("no share {share}"))?;
        if cx.caller.is_console() {
            return Ok(s);
        }
        let me = cx.caller.principal();
        match s.guests.iter().find(|g| &g.principal == me) {
            Some(g) if g.state == Admission::Allowed => Ok(s),
            Some(_) => Err(format!("share {share} has not let you in yet: ask with share_join")),
            None => Err(format!("no share {share}")),
        }
    }

    /// An entry as a reader sees it: the principal's and device's names
    /// from the registry now.
    fn view(cx: &Call, e: &Entry) -> Value {
        let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
        if let Some(p) = &e.principal {
            v["name"] = json!(cx.devices.principal_name(p));
            if let Some(d) = &e.device {
                v["device_name"] = json!(Registry::device_name(&*cx.devices, p, d));
            }
        }
        v
    }

    pub fn read(cx: &Call, share: String, after: Option<u64>, limit: Option<usize>) -> Result<Value, String> {
        admitted(cx, &share)?;
        let page = Threads::at(&cx.ctx.config_dir).page(&share, after.unwrap_or(0), limit.unwrap_or(PAGE_MAX))?;
        Ok(json!({
            "share": share,
            "entries": page.entries.iter().map(|e| view(cx, e)).collect::<Vec<_>>(),
            "more": page.more,
            "last": page.last,
        }))
    }

    /// A post, by the caller, and `shares:changed` to the console and every
    /// guest the share has let in — each new post, so a reader re-reads
    /// `thread_read(after)` instead of polling fast; a retry emits nothing.
    pub fn post(cx: &Call, share: String, id: String, text: String) -> Result<Value, String> {
        let s = admitted(cx, &share)?;
        let device = match cx.caller.device() {
            Device::Paired(d) => Some(d.clone()),
            _ => None,
        };
        let (e, new) = Threads::at(&cx.ctx.config_dir).post(&share, &id, cx.caller.principal(), device.as_ref(), &text, now())?;
        if new {
            changed(&*cx.ctx.host, "post", Some(&share), &allowed(&s), Some(e.seq));
        }
        Ok(view(cx, &e))
    }

    /// The guests a share has let in.
    pub fn allowed(s: &Share) -> Vec<PrincipalId> {
        s.guests.iter().filter(|g| g.state == Admission::Allowed).map(|g| g.principal.clone()).collect()
    }

    /// Every guest a share names.
    pub fn named(s: &Share) -> Vec<PrincipalId> {
        s.guests.iter().map(|g| g.principal.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-thread-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn bret() -> PrincipalId {
        PrincipalId::new("g_bret")
    }

    /// §8: a retried post creates one entry, and a restart (a fresh store
    /// over the same files) keeps the order and the next seq.
    #[test]
    fn a_retried_post_is_one_entry_and_a_restart_keeps_the_order() {
        let d = dir("retry");
        let t = Threads::at(&d);
        let (a, new) = t.post("s_one", "c-1", &bret(), None, "first", NOW).unwrap();
        assert!(new);
        let (again, new) = t.post("s_one", "c-1", &bret(), None, "first", NOW + 5).unwrap();
        assert!(!new, "a retry is not a new post");
        assert_eq!(again, a);
        // Another principal's id is theirs; the same id with other text is refused.
        let (eve, _) = t.post("s_one", "c-1", &PrincipalId::new("g_eve"), None, "mine", NOW).unwrap();
        assert_eq!(eve.seq, 2);
        assert!(t.post("s_one", "c-1", &bret(), None, "changed", NOW).unwrap_err().contains("other text"));
        t.event("s_one", Kind::Joined, Some(&bret()), NOW).unwrap();
        // "Restart": a new store over the same dir.
        let t = Threads::at(&d);
        let (b, _) = t.post("s_one", "c-2", &bret(), None, "second", NOW).unwrap();
        assert_eq!(b.seq, 4);
        let p = t.page("s_one", 0, 100).unwrap();
        assert_eq!(p.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert_eq!((p.more, p.last), (false, 4));
        assert_eq!(p.entries[2].kind, Kind::Joined);
        // Paging: after and limit, in order.
        let p = t.page("s_one", 1, 2).unwrap();
        assert_eq!((p.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), p.more), (vec![2, 3], true));
        // The file is 0600 in a 0700 directory.
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(t.path("s_one").unwrap()).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(d.join(DIR)).unwrap().permissions().mode() & 0o777, 0o700);
        let _ = fs::remove_dir_all(&d);
    }

    /// §4: the character rule at post time, refused never stripped; line
    /// breaks and a leading `/ @ ! #` allowed; 8 KiB.
    #[test]
    fn the_size_and_character_rules_refuse_and_never_strip() {
        assert_eq!(TEXT_MAX, 8 * 1024, "§4: 8 KiB a post");
        let d = dir("rules");
        let t = Threads::at(&d);
        for (i, ok) in ["/clear is just text", "@agent hi", "!ls", "# heading", "two\nlines", &"x".repeat(TEXT_MAX)].iter().enumerate() {
            let (e, _) = t.post("s_one", &format!("ok{i}"), &bret(), None, ok, NOW).unwrap();
            assert_eq!(e.text.as_deref(), Some(*ok));
        }
        let (e, _) = t.post("s_one", "crlf", &bret(), None, "a\r\nb", NOW).unwrap();
        assert_eq!(e.text.as_deref(), Some("a\nb"));
        let before = t.page("s_one", 0, 200).unwrap().last;
        for bad in [&"x".repeat(TEXT_MAX + 1), "tab\there", "esc\u{1b}[2J", "bidi \u{202E}evil", "zw\u{200B}j", "", "   \n "] {
            assert!(t.post("s_one", "bad", &bret(), None, bad, NOW).is_err(), "{bad:?} was taken");
        }
        for id in ["", "has space", &"x".repeat(ID_MAX + 1), "../x"] {
            assert!(t.post("s_one", id, &bret(), None, "fine", NOW).is_err(), "id {id:?} was taken");
        }
        assert_eq!(t.page("s_one", 0, 200).unwrap().last, before, "a refused post took no seq");
        // A share id names a file, so only a share id's shape is one.
        for s in ["../x", "s_A", "x_1", "s_", "s_a/b"] {
            assert!(t.post(s, "c", &bret(), None, "fine", NOW).is_err(), "{s}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    /// An unreadable thread refuses reads and writes and is left alone; a
    /// removed one is gone, and a new thread under it starts at 1.
    #[test]
    fn an_unreadable_thread_is_refused_and_left_and_remove_clears_it() {
        let d = dir("bad");
        let t = Threads::at(&d);
        t.post("s_one", "c", &bret(), None, "hi", NOW).unwrap();
        let path = t.path("s_one").unwrap();
        fs::write(&path, b"{nope").unwrap();
        assert!(t.page("s_one", 0, 10).is_err());
        assert!(t.post("s_one", "c2", &bret(), None, "hi", NOW).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{nope");
        assert!(t.remove("s_one").unwrap());
        assert!(!path.exists());
        assert_eq!(t.post("s_one", "c", &bret(), None, "hi", NOW).unwrap().0.seq, 1);
        let _ = fs::remove_dir_all(&d);
    }

    // ── no path takes thread text to an agent (§4, §8) ───────────────────────

    /// What an agent's side is built from: the MCP server (tools and `wait`),
    /// the agent mail log, and every hook a harness runs (`guard` and the
    /// guidance plugin, the prompt plugin's `agent-hook`, `plan hook`, kimi's
    /// opener). The core and api crates cannot name a serve module at all;
    /// these are the files that could reach the thread by its file or event.
    const AGENT_SIDE: &[(&str, &str)] = &[
        ("mcp.rs", include_str!("../mcp.rs")),
        ("messages.rs", include_str!("../../../worktrees-core/src/messages.rs")),
        ("guidance.rs", include_str!("../../../worktrees-core/src/guidance.rs")),
        ("pending.rs", include_str!("../../../worktrees-core/src/pending.rs")),
        ("plancmd.rs", include_str!("../../../worktrees-core/src/plancmd.rs")),
        ("kimi.rs", include_str!("../../../worktrees-core/src/kimi.rs")),
    ];

    /// The names by which anything could reach a thread: its module, its
    /// rows, its folder, its event.
    fn thread_names(src: &str) -> Vec<&'static str> {
        ["sharethread", "thread_read", "thread_post", "thread_delete", "share-threads", "shares:changed", "Threads::"]
            .into_iter()
            .filter(|n| src.contains(n))
            .collect()
    }

    #[test]
    fn no_agent_side_source_names_the_thread() {
        for (file, src) in AGENT_SIDE {
            assert!(src.len() > 1000, "{file} was read");
            assert_eq!(thread_names(src), Vec::<&str>::new(), "{file} names the thread");
        }
        assert!(AGENT_SIDE[0].1.contains("fn cmd_mcp") && AGENT_SIDE[0].1.contains("\"wait\""), "the MCP server, with its wait");
        // And the thread names no way to an agent: no typing, no pane, no mail.
        // Code only: the doc above says which rows the owner's *Use these* uses.
        let live: String = SRC
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .unwrap()
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(live.contains("pub fn post("), "the slice holds the store");
        for n in ["agents::", "agent_send", "paste_into", "tmux", "send_keys", "messages::append", "messages::report", "prompt::"] {
            assert!(!live.contains(n), "the thread names {n}");
        }
    }

    const SRC: &str = include_str!("sharethread.rs");

    /// Shown red on planted names, so a pass means something.
    #[test]
    fn the_thread_check_fails_on_a_planted_name() {
        for plant in ["crate::serve::sharethread::Threads::at(d)", "\"thread_read\"", "dir.join(\"share-threads\")", "on(\"shares:changed\")"] {
            assert!(!thread_names(&format!("{}\n{plant}\n", AGENT_SIDE[0].1)).is_empty(), "{plant} slipped past");
        }
    }
}

