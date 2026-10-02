//! The place↔place message log — how an agent in one worktree tells an agent in
//! another "done", "stuck", or "which of these did you mean?".
//!
//! Claude sessions already have a bus: `SendMessage` / `ListAgents` addressed by
//! `--name` (the full tmux session name). Codex has nothing — it can neither
//! report back nor be waited on, and before this an orchestrator polled its
//! pane. Both providers speak the `worktrees` MCP server, so the channel lives
//! behind it (`report` / `messages` / `wait`), and it is deliberately
//! PROVIDER-NEUTRAL: `(main)` is just another place, and a Claude↔Codex pair
//! uses it the same way a Codex↔Codex pair does. Claude↔Claude keeps using
//! Claude's own messaging; this is not a replacement for it.
//!
//! **Where: the git COMMON dir** (`<git-common>/worktrees-messages/`). Every
//! worktree of a repo shares it, it is never tracked, and it needs no `$HOME` —
//! a sandboxed agent whose home is not the user's still reaches it. Codex's
//! auto-review sandbox already has the common dir writable (it has to, to
//! commit from a linked worktree), so an agent-side write works either way.
//!
//! **One file per message, never one file rewritten** — the reasoning is
//! `inbox.rs`'s: two agents can report at once, and a shared JSON file would be
//! a read-modify-write race. A message is a create (temp + rename, so a reader
//! never sees half of one) and its identity is its file name.
//!
//! The file name is `<id>.<hex(to)>.json`. Carrying the recipient in the NAME
//! is what keeps `wait` cheap: a reader polling for its own mail filters a
//! `read_dir` and opens only the files addressed to it, never the whole log.
//! Hex because a slug is whatever a directory was called, and `(main)` already
//! has parentheses in it.
//!
//! **Read state is a per-recipient marker, not a field.** `.read/<hex(to)>/<id>`
//! is an empty file. Marking read is a create that is idempotent by nature, so
//! two readers of one place (a claude and the codex beside it, or two `wait`s)
//! cannot race each other into a lost update, and the message file itself is
//! never written after it lands.
//!
//! **Bounded three ways**, because a log that agents write in a loop is a log
//! that can fill a disk: `MAX_TEXT` per message (refused, not truncated — a cut
//! instruction is worse than a refusal the sender can act on), `MAX_COUNT` per
//! project (oldest dropped), and `MAX_AGE_SECS` (expired on the next post, and
//! never served meanwhile). Pruning happens on WRITE, so a reader never deletes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Longest message body, in bytes. A report is a paragraph, not a diff — the
/// diff is in the branch — and every byte of it lands in another model's prompt.
pub const MAX_TEXT: usize = 8 * 1024;
/// Messages kept per project. Beyond this the OLDEST go first.
pub const MAX_COUNT: usize = 500;
/// A week. Long enough to survive a weekend; short enough that a stale "done"
/// cannot be mistaken for today's.
pub const MAX_AGE_SECS: i64 = 7 * 24 * 3600;
/// Longest place slug accepted as `from` / `to`.
const MAX_SLUG: usize = 256;

/// The directory name inside the git common dir.
pub const DIR_NAME: &str = "worktrees-messages";

/// One message. `from` is whatever the POSTER's server resolved its own place
/// to — never an argument a model supplied — so a reader can trust it as far as
/// it trusts the machine.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub text: String,
    /// Unix seconds.
    pub created: i64,
    /// The message this one answers, when it answers one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// `send` when the text was also TYPED into the recipient's pane — this
    /// copy is then the record, and is filed already-read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

/// A message plus whether its recipient has read it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Stored {
    #[serde(flatten)]
    pub msg: Message,
    pub read: bool,
}

/// `<git-common>/worktrees-messages`.
pub fn dir(git_common: &Path) -> PathBuf {
    git_common.join(DIR_NAME)
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// An id is `<ms:013>-<pid:x>-<seq:08x>`: sortable by time, unique across
/// processes (pid) and across threads in one (seq).
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn new_id(now_ms: i64) -> String {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{:013}-{:x}-{:08x}", now_ms.max(0), std::process::id(), seq & 0xffff_ffff)
}

/// The id's own millisecond stamp — ages are read from the NAME, so pruning
/// never opens a file.
fn id_ms(id: &str) -> Option<i64> {
    id.split('-').next()?.parse().ok()
}

/// `(id, hex(to), path)` for every message file in `dir`, oldest first.
fn entries(dir: &Path) -> Vec<(String, String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(String, String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let stem = name.strip_suffix(".json")?;
            let (id, to) = stem.split_once('.')?;
            valid_id(id).then(|| (id.to_string(), to.to_string(), e.path()))
        })
        .collect();
    v.sort();
    v
}

fn marker(dir: &Path, to_hex: &str, id: &str) -> PathBuf {
    dir.join(".read").join(to_hex).join(id)
}

fn check_slug(what: &str, s: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("{what} is required"));
    }
    if s.len() > MAX_SLUG || s.chars().any(char::is_control) {
        return Err(format!("{what} is not a place name"));
    }
    Ok(())
}

/// Post a message. Refuses rather than trims: an empty body, one over
/// `MAX_TEXT`, a NUL, or a `reply_to` that names no message in the log.
/// Prunes the log afterwards (see the module note).
pub fn post(
    dir: &Path,
    from: &str,
    to: &str,
    text: &str,
    reply_to: Option<&str>,
    via: Option<&str>,
    now_ms: i64,
) -> Result<Message, String> {
    post_into(dir, dir, from, to, text, reply_to, via, now_ms)
}

/// `post`, with the log `reply_to` must name a message in given separately
/// (cross-project §4.2). A message from ANOTHER repository is filed in the
/// RECIPIENT's log, so the message it answers sits in the REPLIER's own log —
/// where it arrived. Checked against the log being written, an A→B→A reply
/// could never name what it answers. Same-project posts pass `dir` twice
/// (`post`), which is exactly the old check.
#[allow(clippy::too_many_arguments)]
pub fn post_into(
    dir: &Path,
    reply_log: &Path,
    from: &str,
    to: &str,
    text: &str,
    reply_to: Option<&str>,
    via: Option<&str>,
    now_ms: i64,
) -> Result<Message, String> {
    check_slug("from", from)?;
    check_slug("to", to)?;
    if text.trim().is_empty() {
        return Err("text is empty".into());
    }
    if text.len() > MAX_TEXT {
        return Err(format!(
            "text is {} bytes; the limit is {MAX_TEXT}. Say what changed and where — the branch holds the rest.",
            text.len()
        ));
    }
    if text.contains('\0') {
        return Err("text contains a NUL byte".into());
    }
    if let Some(r) = reply_to {
        if !valid_id(r) || !entries(reply_log).iter().any(|(id, _, _)| id == r) {
            return Err(format!("reply_to names no message in the log: {r}"));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let msg = Message {
        id: new_id(now_ms),
        from: from.to_string(),
        to: to.to_string(),
        text: text.to_string(),
        created: now_ms.div_euclid(1000),
        reply_to: reply_to.map(str::to_string),
        via: via.map(str::to_string),
    };
    let body = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
    let name = format!("{}.{}.json", msg.id, hex(to));
    // Temp + rename in the same directory: atomic, and never a partial read.
    let tmp = dir.join(format!(".{}.tmp", msg.id));
    let dst = dir.join(&name);
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", dst.display())
    })?;
    prune(dir, now_ms);
    Ok(msg)
}

/// Drop what has expired, then the oldest beyond `MAX_COUNT`, then any read
/// marker whose message is gone and any temp file a crashed writer left. Never
/// errors: a prune that fails is retried by the next post.
pub fn prune(dir: &Path, now_ms: i64) {
    prune_listed(dir, now_ms, entries(dir));
}

/// `prune` over a listing taken earlier — which, with other writers about, is
/// always somewhat STALE by the time it is acted on. Split out so a test can
/// hand it one.
fn prune_listed(dir: &Path, now_ms: i64, all: Vec<(String, String, PathBuf)>) {
    let cutoff = now_ms - MAX_AGE_SECS * 1000;
    let (expired, live): (Vec<_>, Vec<_>) =
        all.into_iter().partition(|(id, _, _)| id_ms(id).is_none_or(|ms| ms < cutoff));
    let excess = live.len().saturating_sub(MAX_COUNT);
    for (_, _, p) in expired.iter().chain(live.iter().take(excess)) {
        let _ = std::fs::remove_file(p);
    }
    // A read marker goes only when its message file is GONE — asked of the
    // filesystem at the moment of removal, never of the listing above. The
    // listing is stale by construction: a post and an ack can land between it
    // and this loop, and a marker judged against it would be a LIVE one
    // deleted, bringing that message back unread. The marker's own path names
    // the message's (`.read/<hex(to)>/<id>` ↔ `<id>.<hex(to)>.json`), so this
    // is one stat per marker.
    if let Ok(rd) = std::fs::read_dir(dir.join(".read")) {
        for who in rd.flatten() {
            let to_hex = who.file_name().to_string_lossy().into_owned();
            if let Ok(marks) = std::fs::read_dir(who.path()) {
                for m in marks.flatten() {
                    let id = m.file_name().to_string_lossy().into_owned();
                    if !dir.join(format!("{id}.{to_hex}.json")).exists() {
                        let _ = std::fs::remove_file(m.path());
                    }
                }
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let old = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|d| d.as_secs() > 60);
            if name.starts_with('.') && name.ends_with(".tmp") && old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Every live message addressed to `to`, oldest first, each with its read
/// state. Opens only the files whose NAME says they are for `to`. Expired ones
/// are skipped (the next post deletes them) and a file that does not parse is
/// skipped rather than fatal.
pub fn for_place(dir: &Path, to: &str, now_ms: i64) -> Vec<Stored> {
    let want = hex(to);
    let cutoff = now_ms - MAX_AGE_SECS * 1000;
    entries(dir)
        .into_iter()
        .filter(|(id, h, _)| *h == want && id_ms(id).is_some_and(|ms| ms >= cutoff))
        .filter_map(|(id, h, p)| {
            // Nothing `post` wrote can be this big; a file that is was put
            // here by something else, and is not read into memory to find out.
            if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(u64::MAX) > (MAX_TEXT + 1024) as u64 {
                return None;
            }
            let msg: Message = serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            // The name is the index; the body is the record. If they disagree
            // the file was not written by `post`, and it is not served.
            if msg.id != id || msg.to != to {
                return None;
            }
            let read = marker(dir, &h, &id).exists();
            Some(Stored { msg, read })
        })
        .collect()
}

/// The unread subset of `for_place`, optionally only from `from`.
pub fn unread(dir: &Path, to: &str, from: Option<&str>, now_ms: i64) -> Vec<Message> {
    for_place(dir, to, now_ms)
        .into_iter()
        .filter(|s| !s.read && from.is_none_or(|f| s.msg.from == f))
        .map(|s| s.msg)
        .collect()
}

/// Mark `ids` read FOR `to`. Idempotent: a marker that already exists is the
/// same answer, which is what lets two readers ack at once.
pub fn ack(dir: &Path, to: &str, ids: &[String]) -> Result<(), String> {
    let h = hex(to);
    let d = dir.join(".read").join(&h);
    std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    for id in ids.iter().filter(|i| valid_id(i)) {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(marker(dir, &h, id)) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{id}: {e}")),
        }
    }
    Ok(())
}

/// Milliseconds since the epoch, for the callers that post for real.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("wtmsg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Tmp(d)
    }

    const T0: i64 = 1_790_000_000_000;

    /// A→B→A across two logs: B's reply is filed in A's log and names a
    /// message that sits in B's own log. `post` would look for it in A's.
    #[test]
    fn a_cross_log_reply_is_checked_against_the_repliers_own_log() {
        let (a, b) = (tmp("xa"), tmp("xb"));
        // alpha's lane → beta's (main): filed in beta's log, `to` bare.
        let q = post_into(&b.0, &a.0, "alpha:lane", "(main)", "which branch?", None, None, T0).unwrap();
        assert_eq!(for_place(&b.0, "(main)", T0 + 1).len(), 1);
        assert!(for_place(&a.0, "(main)", T0 + 1).is_empty());
        // beta's (main) answers into alpha's log; the question is in BETA's.
        let ans = post_into(&a.0, &b.0, "beta:(main)", "lane", "feat-x", Some(&q.id), None, T0 + 5).unwrap();
        assert_eq!(ans.reply_to.as_deref(), Some(q.id.as_str()));
        assert_eq!(for_place(&a.0, "lane", T0 + 6)[0].msg.from, "beta:(main)");
        // The same reply checked the old way — against the log being written —
        // cannot find what it answers.
        assert!(post(&a.0, "beta:(main)", "lane", "feat-x", Some(&q.id), None, T0 + 7).unwrap_err().contains("reply_to names no message"));
    }

    #[test]
    fn a_message_round_trips_to_its_recipient_only() {
        let t = tmp("rt");
        let m = post(&t.0, "feat-a", "(main)", "done: tests green", None, None, T0).unwrap();
        assert_eq!(m.created, T0 / 1000);
        let got = for_place(&t.0, "(main)", T0 + 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].msg, m);
        assert!(!got[0].read);
        assert!(for_place(&t.0, "feat-a", T0 + 1).is_empty(), "the sender's own inbox is not the recipient's");
        // The recipient is in the NAME, so a reader filters without opening.
        let names: Vec<String> = std::fs::read_dir(&t.0)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".json"))
            .collect();
        assert_eq!(names, vec![format!("{}.{}.json", m.id, hex("(main)"))]);
    }

    /// Two writers at once, from two threads (same pid — the sequence number
    /// is what separates them) — every message must land, none overwritten.
    #[test]
    fn two_concurrent_writers_lose_nothing() {
        let t = tmp("conc");
        let d = t.0.clone();
        let d2 = t.0.clone();
        let a = std::thread::spawn(move || {
            for i in 0..40 {
                post(&d, "a", "(main)", &format!("a{i}"), None, None, T0).unwrap();
            }
        });
        let b = std::thread::spawn(move || {
            for i in 0..40 {
                post(&d2, "b", "(main)", &format!("b{i}"), None, None, T0).unwrap();
            }
        });
        a.join().unwrap();
        b.join().unwrap();
        let got = for_place(&t.0, "(main)", T0);
        assert_eq!(got.len(), 80, "a same-millisecond post must not overwrite another");
        // Per writer, the order it posted in is the order it is read in.
        let a_seq: Vec<String> = got.iter().filter(|s| s.msg.from == "a").map(|s| s.msg.text.clone()).collect();
        assert_eq!(a_seq, (0..40).map(|i| format!("a{i}")).collect::<Vec<_>>());
    }

    #[test]
    fn messages_are_served_oldest_first() {
        let t = tmp("order");
        post(&t.0, "x", "y", "second", None, None, T0 + 5000).unwrap();
        post(&t.0, "x", "y", "first", None, None, T0).unwrap();
        post(&t.0, "x", "y", "third", None, None, T0 + 9000).unwrap();
        let texts: Vec<String> = for_place(&t.0, "y", T0 + 10_000).into_iter().map(|s| s.msg.text).collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    #[test]
    fn text_is_bounded_and_refused_rather_than_cut() {
        let t = tmp("bounds");
        assert!(post(&t.0, "x", "y", &"a".repeat(MAX_TEXT), None, None, T0).is_ok());
        let e = post(&t.0, "x", "y", &"a".repeat(MAX_TEXT + 1), None, None, T0).unwrap_err();
        assert!(e.contains("limit"), "{e}");
        assert!(post(&t.0, "x", "y", "   ", None, None, T0).is_err(), "an empty report says nothing");
        assert!(post(&t.0, "x", "y", "a\0b", None, None, T0).is_err());
        assert!(post(&t.0, "", "y", "hi", None, None, T0).is_err(), "from is required");
        assert!(post(&t.0, "x", "a\nb", "hi", None, None, T0).is_err(), "a slug has no newline");
        assert_eq!(for_place(&t.0, "y", T0).len(), 1, "only the in-bounds one landed");
    }

    #[test]
    fn the_count_is_capped_by_dropping_the_oldest() {
        let t = tmp("cap");
        for i in 0..(MAX_COUNT + 3) {
            post(&t.0, "x", "y", &format!("m{i}"), None, None, T0 + i as i64).unwrap();
        }
        let got = for_place(&t.0, "y", T0 + 10_000);
        assert_eq!(got.len(), MAX_COUNT);
        assert_eq!(got[0].msg.text, "m3", "the three oldest went");
    }

    #[test]
    fn an_expired_message_is_not_served_and_the_next_post_deletes_it() {
        let t = tmp("age");
        let old = post(&t.0, "x", "y", "old", None, None, T0).unwrap();
        ack(&t.0, "y", std::slice::from_ref(&old.id)).unwrap();
        let later = T0 + (MAX_AGE_SECS + 1) * 1000;
        assert!(for_place(&t.0, "y", later).is_empty(), "a week-old message is not today's news");
        post(&t.0, "x", "y", "new", None, None, later).unwrap();
        let files = entries(&t.0);
        assert_eq!(files.len(), 1, "expired file deleted on write: {files:?}");
        assert!(!marker(&t.0, &hex("y"), &old.id).exists(), "its read marker goes with it");
    }

    /// Read state is PER RECIPIENT and idempotent: two readers acking the same
    /// message is not a conflict, and the message file itself never changes.
    #[test]
    fn ack_is_per_recipient_and_idempotent() {
        let t = tmp("ack");
        let m1 = post(&t.0, "x", "y", "one", None, None, T0).unwrap();
        let m2 = post(&t.0, "x", "y", "two", None, None, T0 + 1).unwrap();
        post(&t.0, "x", "z", "for z", None, None, T0 + 2).unwrap();
        let before = std::fs::read_to_string(entries(&t.0)[0].2.clone()).unwrap();
        ack(&t.0, "y", std::slice::from_ref(&m1.id)).unwrap();
        ack(&t.0, "y", std::slice::from_ref(&m1.id)).unwrap(); // a second reader
        let un = unread(&t.0, "y", None, T0 + 5);
        assert_eq!(un, vec![m2.clone()]);
        assert_eq!(unread(&t.0, "z", None, T0 + 5).len(), 1, "z's mail is untouched by y's ack");
        assert_eq!(std::fs::read_to_string(entries(&t.0)[0].2.clone()).unwrap(), before);
        assert_eq!(unread(&t.0, "y", Some("nobody"), T0 + 5).len(), 0, "the from filter");
        assert_eq!(unread(&t.0, "y", Some("x"), T0 + 5).len(), 1);
    }

    /// The race: prune lists, then another writer posts and the recipient acks,
    /// then prune sweeps markers. A marker for a message the stale listing
    /// never saw is LIVE and must survive — or that message comes back unread.
    #[test]
    fn prune_with_a_stale_listing_keeps_a_live_read_marker() {
        let t = tmp("race");
        post(&t.0, "x", "y", "old", None, None, T0).unwrap();
        let stale = entries(&t.0);
        let m = post(&t.0, "x", "y", "new", None, None, T0 + 1).unwrap();
        ack(&t.0, "y", std::slice::from_ref(&m.id)).unwrap();
        prune_listed(&t.0, T0 + 2, stale);
        assert!(marker(&t.0, &hex("y"), &m.id).exists(), "a live read marker was deleted");
        assert!(unread(&t.0, "y", None, T0 + 3).iter().all(|u| u.id != m.id));
    }

    #[test]
    fn reply_to_must_name_a_message_in_the_log() {
        let t = tmp("reply");
        let q = post(&t.0, "a", "b", "which API?", None, None, T0).unwrap();
        let r = post(&t.0, "b", "a", "the v2 one", Some(&q.id), None, T0 + 1).unwrap();
        assert_eq!(r.reply_to.as_deref(), Some(q.id.as_str()));
        assert!(post(&t.0, "b", "a", "x", Some("0000000000000-1-00000000"), None, T0).is_err());
        assert!(post(&t.0, "b", "a", "x", Some("../etc"), None, T0).is_err());
    }

    /// A file dropped by hand whose body disagrees with its name is not served
    /// — the name is only an index, and `to` in particular must not be forged
    /// by renaming.
    #[test]
    fn a_file_whose_body_disagrees_with_its_name_is_skipped() {
        let t = tmp("forge");
        let m = post(&t.0, "x", "y", "real", None, None, T0).unwrap();
        let body = std::fs::read_to_string(&entries(&t.0)[0].2).unwrap();
        std::fs::write(t.0.join(format!("{}.{}.json", m.id, hex("z"))), body).unwrap();
        assert!(for_place(&t.0, "z", T0).is_empty());
        std::fs::write(t.0.join(format!("{T0:013}-1-00000000.{}.json", hex("y"))), "{ nope").unwrap();
        assert_eq!(for_place(&t.0, "y", T0).len(), 1, "junk is skipped, not fatal");
        // An oversized file is skipped by its SIZE, even when its body is valid.
        let mut big = m.clone();
        big.id = format!("{:013}-1-00000001", T0);
        big.text = "x".repeat(MAX_TEXT + 2048);
        std::fs::write(t.0.join(format!("{}.{}.json", big.id, hex("y"))), serde_json::to_string(&big).unwrap()).unwrap();
        assert_eq!(for_place(&t.0, "y", T0).len(), 1, "an oversized file is not served");
    }
}
