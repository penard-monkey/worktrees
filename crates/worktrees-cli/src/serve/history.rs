//! A shared session's FULL history (group chat PR C,
//! `docs/proposals/group-chat.md` §3.2, §3.3): `shared_turns` and
//! `shared_tool_call`, the rows a guest (and the owner's console) read a
//! share's bound transcript through.
//!
//! **One index per transcript, built once and extended as the file grows.**
//! `agent_turns` reads the last 2 MiB and 20 turns; a shared history is the
//! whole file, which on this machine reaches 393 MB. So the bytes are read
//! ONCE, line by line, through `turns::Indexer` — the same parser every turn
//! read uses — into each turn's byte spans and the turn each tool call is
//! in; a page then reads only its own turns' spans. The index is held in
//! memory, keyed by (harness, transcript), with the length it was read to: a
//! later read reads the bytes past that and nothing else. A file that got
//! SHORTER, or is another file now (its inode), is indexed again from the
//! start. Keyed by the transcript and not by the share, because two
//! bindings can name one file (an ended binding and a *Share again* of the
//! same resumed id) and the index holds offsets only; each binding's limit
//! is applied on every read, never baked into the index.
//!
//! **An ended binding stops at its offset** (§3.3): a span at or past
//! `ended_offset` is never read, so a resumed old id that grows shows a guest
//! nothing new — and a turn the index had joined a later line to (a command's
//! output) is read without it.
//!
//! **A page** is newest first, at most [`PAGE_TURNS_MAX`] turns and
//! [`PAGE_BYTES`] of JSON, with `next_before` as the cursor. Each turn is
//! whole up to [`turns::TURN_TEXT_MAX`] characters a string (`agent_turn_text`'s
//! cap: ~5x the longest reply measured on this machine), and one that is cut
//! says so with its counts. What a turn carries is `agent_turns`' reading:
//! no tool output, no file contents, no `api-request-blob`, the same control
//! and invisible characters removed.

use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use worktrees_core::turns;

use super::shares::{Binding, BindingState, Share};

/// JSON bytes one page carries, at most: a quarter of the 8 MiB outbox
/// (`budget.rs`), so a page, a stream's 2 MiB and the largest other reply
/// (`read_file`'s 5.72 MiB) never meet the cap together with a second page.
pub const PAGE_BYTES: usize = 2 * 1024 * 1024;

/// Turns one page carries, at most, and when the client names none.
pub const PAGE_TURNS_MAX: usize = 50;
pub const PAGE_TURNS_DEFAULT: usize = 20;

/// What a turn too big for a page is cut to before it is dropped steps-first:
/// `agent_turns`' own cap.
const TIGHT_TEXT_MAX: usize = turns::TEXT_MAX;

/// Bytes read per `read_at` while indexing.
const CHUNK: usize = 8 * 1024 * 1024;

/// A line longer than this is skipped by the index, never held: the longest
/// lines are Claude's `api-request-blob`s (~100 KB each) and images a Read
/// was shown (a few MB of base64); a turn line past it is not one a reader
/// could carry anyway. Its bytes still count as read.
const LINE_MAX: usize = 64 * 1024 * 1024;

/// One transcript's index.
pub struct Index {
    indexer: turns::Indexer,
    /// Each turn's spans, in the order they were found.
    spans: Vec<Vec<(u64, u64)>>,
    /// The turn each call id is in.
    calls: HashMap<String, usize>,
    /// Bytes read so far: every complete line before this is indexed.
    built_to: u64,
    /// The file it was read from (device, inode).
    file: (u64, u64),
    /// Bytes read from the file to index it, in all — what a test asserts
    /// to show an extension reads only what is new.
    pub index_bytes: u64,
    /// A line over [`LINE_MAX`] that is still being skipped: its bytes up to
    /// here were read, and nothing of it indexed.
    skipping: bool,
}

impl Index {
    fn new(harness: &str, file: (u64, u64)) -> Option<Index> {
        Some(Index { indexer: turns::Indexer::new(harness)?, spans: Vec::new(), calls: HashMap::new(), built_to: 0, file, index_bytes: 0, skipping: false })
    }

    /// Read `f` from where the index stopped to `len`, complete lines only:
    /// an incomplete last line is read again, whole, next time.
    fn extend(&mut self, f: &File, len: u64) -> Result<(), String> {
        let mut buf = vec![0u8; CHUNK];
        let mut carry: Vec<u8> = Vec::new();
        // The first byte of the line being read.
        let mut at = self.built_to;
        let mut from = self.built_to;
        while from < len {
            let want = ((len - from) as usize).min(CHUNK);
            let n = f.read_at(&mut buf[..want], from).map_err(|e| format!("the transcript could not be read: {e}"))?;
            if n == 0 {
                break;
            }
            self.index_bytes += n as u64;
            let mut seg = 0;
            for (p, _) in buf[..n].iter().enumerate().filter(|(_, b)| **b == b'\n') {
                let end = from + p as u64 + 1;
                if self.skipping {
                    self.skipping = false;
                } else if carry.is_empty() {
                    self.line(&buf[seg..p], at, end);
                } else {
                    carry.extend_from_slice(&buf[seg..p]);
                    let whole = std::mem::take(&mut carry);
                    self.line(&whole, at, end);
                }
                at = end;
                seg = p + 1;
            }
            if !self.skipping {
                carry.extend_from_slice(&buf[seg..n]);
                if carry.len() > LINE_MAX {
                    carry = Vec::new();
                    self.skipping = true;
                }
            }
            from += n as u64;
        }
        // Skipping, the line's bytes so far are spent; otherwise the
        // incomplete line is read again.
        self.built_to = if self.skipping { from } else { at };
        Ok(())
    }

    fn line(&mut self, line: &[u8], start: u64, end: u64) {
        let Ok(text) = std::str::from_utf8(line) else { return };
        for sp in self.indexer.feed(text, start, end) {
            if sp.begins {
                self.spans.push(Vec::new());
            }
            if let Some(s) = self.spans.get_mut(sp.turn) {
                match s.last_mut() {
                    Some(last) if last.1 == sp.start => last.1 = sp.end,
                    _ => s.push((sp.start, sp.end)),
                }
            }
        }
        for (id, turn) in self.indexer.take_calls() {
            self.calls.insert(id, turn);
        }
    }

    /// Turns whose first byte is below `limit`.
    fn visible(&self, limit: u64) -> usize {
        self.spans.partition_point(|s| s.first().is_some_and(|f| f.0 < limit))
    }

    /// Turn `i`'s lines below `limit`: a span past it left out, and one
    /// across it cut there (spans of adjacent lines are kept as one, so the
    /// lines of a turn an ended session went on with share its span).
    fn lines(&self, f: &File, i: usize, limit: u64) -> Result<Vec<String>, String> {
        let mut spans: Vec<(u64, u64)> = self.spans[i].iter().filter(|(s, _)| *s < limit).map(|&(s, e)| (s, e.min(limit))).collect();
        spans.sort();
        let mut bytes = Vec::new();
        for (s, e) in spans {
            let mut b = vec![0u8; (e - s) as usize];
            f.read_exact_at(&mut b, s).map_err(|e| format!("the transcript could not be read: {e}"))?;
            bytes.extend_from_slice(&b);
        }
        Ok(turns::span_lines(&bytes))
    }
}

/// (the config dir whose shares name it, harness, transcript).
type Key = (PathBuf, String, PathBuf);

/// The indexes serve holds: one per bound transcript.
#[derive(Default)]
pub struct Indexes {
    map: Mutex<HashMap<Key, Arc<Mutex<Index>>>>,
}

/// Serve's indexes. A test that asserts on one index's counters reaches it by
/// its own scratch transcript's path, which no other test shares.
pub static INDEXES: std::sync::LazyLock<Indexes> = std::sync::LazyLock::new(Indexes::default);

impl Indexes {
    /// The index of `harness`'s `transcript`, read to `limit` (or the file's
    /// end, if shorter). Built on first use; extended past what it was read to;
    /// built again when the file shrank or is another file.
    pub fn read_to(&self, config: &Path, harness: &str, transcript: &Path, limit: Option<u64>) -> Result<(Arc<Mutex<Index>>, File, u64), String> {
        if !turns::history_reads(harness) {
            return Err(format!("{harness} sessions have no transcript reader"));
        }
        let f = File::open(transcript).map_err(|_| "this session's transcript is gone from the host".to_string())?;
        let m = f.metadata().map_err(|e| e.to_string())?;
        let id = (m.dev(), m.ino());
        let len = m.len();
        let key = (config.to_path_buf(), harness.to_string(), transcript.to_path_buf());
        let ix = {
            let mut map = self.map.lock().unwrap();
            map.entry(key).or_insert_with(|| Arc::new(Mutex::new(Index::new(harness, id).expect("history_reads was checked")))).clone()
        };
        {
            let mut g = ix.lock().unwrap();
            if g.file != id || len < g.built_to {
                let spent = g.index_bytes;
                *g = Index::new(harness, id).expect("history_reads was checked");
                g.index_bytes = spent;
            }
            let to = limit.map_or(len, |l| l.min(len));
            if to > g.built_to {
                g.extend(&f, to)?;
            }
        }
        Ok((ix, f, limit.map_or(len, |l| l.min(len))))
    }

    /// Drop every index of `config`'s that no binding in its `shares` names
    /// any more.
    pub fn keep_only(&self, config: &Path, shares: &[Share]) {
        let named: Vec<(&str, &Path)> = shares
            .iter()
            .flat_map(|s| s.binding.iter().chain(s.ended.iter()))
            .map(|b| (b.harness.as_str(), b.transcript.as_path()))
            .collect();
        self.map.lock().unwrap().retain(|(c, h, p), _| c != config || named.iter().any(|(nh, np)| nh == h && np == p));
    }

    #[cfg(test)]
    pub fn index_bytes(&self, config: &Path, harness: &str, transcript: &Path) -> Option<u64> {
        let map = self.map.lock().unwrap();
        map.get(&(config.to_path_buf(), harness.to_string(), transcript.to_path_buf())).map(|i| i.lock().unwrap().index_bytes)
    }
}

/// The byte a binding is read to: an ended one's offset, an active one's
/// file end.
fn limit_of(b: &Binding) -> Option<u64> {
    match b.state {
        BindingState::Ended => Some(b.ended_offset.unwrap_or(0)),
        BindingState::Active => None,
    }
}

/// One turn as a page carries it: `turns::Turn`'s fields plus `seq` (its
/// place in the history, the cursor's unit), and, when anything was cut,
/// `truncated` with `chars` (its text in all) and `kept`.
fn turn_json(seq: usize, t: &turns::Turn, chars: usize) -> Value {
    let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
    if let Value::Object(o) = &mut v {
        o.insert("seq".into(), json!(seq));
        if t.truncated {
            o.insert("chars".into(), json!(chars));
            o.insert("kept".into(), json!(turns::text_chars(t)));
        }
    }
    v
}

fn size(v: &Value) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

/// A turn that alone is over the page: cut to `agent_turns`' cap, then its
/// steps dropped from the end until it fits. Always `truncated`.
fn shrink(seq: usize, mut t: turns::Turn, chars: usize) -> Value {
    t = turns::clip_whole(t, TIGHT_TEXT_MAX);
    t.truncated = true;
    let mut v = turn_json(seq, &t, chars);
    while size(&v) > PAGE_BYTES && !t.steps.is_empty() {
        let drop = (t.steps.len() / 2).max(1);
        t.steps.truncate(t.steps.len() - drop);
        v = turn_json(seq, &t, chars);
    }
    v
}

/// The binding a read names: `binding` absent is the share's current one;
/// `n` is `ended[n]`, as `shares_list` lists them (oldest first).
fn binding_of(s: &Share, binding: Option<usize>) -> Result<&Binding, String> {
    match binding {
        None => s.binding.as_ref().ok_or_else(|| "nothing is bound on this share".to_string()),
        Some(n) => s.ended.get(n).ok_or_else(|| format!("this share has no ended binding {n}")),
    }
}

/// `shared_turns`' answer for binding `b`.
pub fn page(config: &Path, b: &Binding, before: Option<usize>, limit: Option<usize>) -> Result<Value, String> {
    let (ix, f, to) = INDEXES.read_to(config, &b.harness, &b.transcript, limit_of(b))?;
    let ix = ix.lock().unwrap();
    let total = ix.visible(to);
    let hi = before.map_or(total, |b| b.min(total));
    let want = limit.unwrap_or(PAGE_TURNS_DEFAULT).clamp(1, PAGE_TURNS_MAX);
    let mut out: Vec<Value> = Vec::new();
    let mut bytes = 0usize;
    let mut full = false;
    let mut next = hi;
    let mut i = hi;
    'turns: while i > 0 && out.len() < want {
        i -= 1;
        let lines = ix.lines(&f, i, to)?;
        let mut got: Vec<Value> = Vec::new();
        for (t, chars) in turns::history_turns(&b.harness, &lines, turns::TURN_TEXT_MAX).into_iter().rev() {
            let v = turn_json(i, &t, chars);
            got.push(if size(&v) > PAGE_BYTES { shrink(i, t, chars) } else { v });
        }
        let cost: usize = got.iter().map(size).sum::<usize>() + got.len();
        if !out.is_empty() && bytes + cost > PAGE_BYTES {
            full = true;
            break 'turns;
        }
        bytes += cost;
        out.extend(got);
        next = i;
    }
    Ok(json!({
        "harness": b.harness,
        "state": b.state,
        "turns": out,
        "total": total,
        "next_before": next,
        "more": next > 0,
        "page_full": full,
        "page_bytes": PAGE_BYTES,
        "turn_text_max": turns::TURN_TEXT_MAX,
    }))
}

/// `shared_tool_call`'s answer for binding `b`: the call's input as
/// `agent_tool_call` gives it, read from the turn the index found it in and
/// only below the binding's limit.
pub fn tool_call(config: &Path, b: &Binding, call_id: &str) -> Result<Value, String> {
    let gone = || "not found: no such call in this shared session".to_string();
    let (ix, f, to) = INDEXES.read_to(config, &b.harness, &b.transcript, limit_of(b))?;
    let ix = ix.lock().unwrap();
    let turn = *ix.calls.get(call_id).ok_or_else(gone)?;
    if turn >= ix.visible(to) {
        return Err(gone());
    }
    let lines = ix.lines(&f, turn, to)?;
    let c = turns::history_tool_call(&b.harness, &lines, call_id).ok_or_else(gone)?;
    Ok(json!({ "tool": c.tool, "input": c.input, "truncated": c.truncated, "bytes": c.bytes, "call_id": call_id, "harness": b.harness }))
}

/// The bodies of the two rows (`dispatch.rs`), after the resolver's
/// `share(share)` check: the share names the caller (or the caller is the
/// console). A guest must also be ALLOWED on it.
pub mod rows {
    use super::*;
    use crate::serve::dispatch::Call;
    use crate::serve::shares::{Admission, Shares};

    fn share(cx: &Call, id: &str) -> Result<Share, String> {
        let f = Shares::at(&cx.ctx.config_dir).read()?;
        INDEXES.keep_only(&cx.ctx.config_dir, &f.shares);
        let s = f.shares.into_iter().find(|s| s.id == id).ok_or_else(|| format!("no share {id}"))?;
        if !cx.caller.is_console() {
            let me = cx.caller.principal();
            match s.guests.iter().find(|g| &g.principal == me).map(|g| g.state) {
                Some(Admission::Allowed) => {}
                Some(_) => return Err("not let in yet: the owner has not allowed you on this share".into()),
                None => return Err(format!("no share {id}")),
            }
        }
        Ok(s)
    }

    pub fn turns(cx: &Call, id: String, binding: Option<usize>, before: Option<usize>, limit: Option<usize>) -> Result<Value, String> {
        let s = share(cx, &id)?;
        let mut v = page(&cx.ctx.config_dir, binding_of(&s, binding)?, before, limit)?;
        v["share"] = json!(id);
        v["binding"] = json!(binding);
        Ok(v)
    }

    pub fn tool_call(cx: &Call, id: String, binding: Option<usize>, call_id: String) -> Result<Value, String> {
        let s = share(cx, &id)?;
        super::tool_call(&cx.ctx.config_dir, binding_of(&s, binding)?, &call_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::dispatch::{CONSOLE_GUEST_ROWS, GUEST_ROWS, TABLE};
    use crate::serve::resolve::{self as res};
    use crate::serve::shares::serve_tests::Host;
    use crate::serve::shares::{place_dir, TEST_LIVE};
    use std::io::Write;
    use worktrees_api::agents::LiveSession;
    use worktrees_core::authz::Capability;

    fn user(uuid: &str, text: &str) -> String {
        json!({"type": "user", "uuid": uuid, "timestamp": "2026-11-12T10:00:00Z", "origin": {"kind": "human"},
            "message": {"role": "user", "content": text}}).to_string() + "\n"
    }

    fn reply(text: &str) -> String {
        json!({"type": "assistant", "uuid": "a", "timestamp": "2026-11-12T10:00:01Z",
            "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}).to_string() + "\n"
    }

    fn call(id: &str, command: &str) -> String {
        json!({"type": "assistant", "uuid": "a", "timestamp": "2026-11-12T10:00:01Z",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}]}}).to_string() + "\n"
    }

    fn output(id: &str, out: &str) -> String {
        json!({"type": "user", "uuid": "r", "timestamp": "2026-11-12T10:00:02Z",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": out}]}}).to_string() + "\n"
    }

    /// Claude's system prompt and CLAUDE.md, as it writes them: quoting
    /// JSON that looks like a user entry, as a real prompt does.
    fn blob(marker: &str, pad: usize) -> String {
        let text = format!("{marker} {{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{marker}\"}}}} {}", "p".repeat(pad));
        json!({"type": "api-request-blob", "message": {"role": "user", "content": [{"type": "text", "text": text}]}}).to_string() + "\n"
    }

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    fn append(path: &Path, text: &str) {
        std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
    }

    /// A share of place `feat` of Host's project `a`, bound to `session`
    /// whose transcript is `text`, with Bret allowed on it.
    fn shared(h: &Host, session: &str, text: &str) -> (String, PathBuf) {
        let t = h.top.join(format!("{session}.jsonl"));
        write(&t, text);
        h.live("feat", session, 0);
        let bret = Host::principal(&h.bret);
        let id = h.console("share_create", json!({"repo": h.a, "slug": "feat", "guests": [bret]})).unwrap()["id"].as_str().unwrap().to_string();
        h.console("share_allow", json!({"share": id, "principal": bret})).unwrap();
        (id, t)
    }

    fn says(v: &Value) -> Vec<String> {
        v["turns"].as_array().unwrap().iter().map(|t| t["user"].as_str().unwrap_or("").to_string()).collect()
    }

    /// The two rows are guest rows the console makes too, and nothing else is.
    #[test]
    fn the_history_rows_are_guest_rows_the_console_may_make() {
        for n in ["shared_turns", "shared_tool_call"] {
            let row = TABLE.iter().find(|r| r.name == n).unwrap();
            assert_eq!(row.cap, Some(Capability::Guest), "{n}");
            assert!(GUEST_ROWS.contains(&n) && CONSOLE_GUEST_ROWS.contains(&n), "{n}");
        }
        for n in CONSOLE_GUEST_ROWS {
            assert!(GUEST_ROWS.contains(n), "{n} is not a guest row");
        }
        let h = Host::new("hist-console");
        let (id, _) = shared(&h, "s-1", &user("u1", "hello"));
        assert_eq!(says(&h.console("shared_turns", json!({"share": id})).unwrap()), vec!["hello"]);
        // The console still makes no other guest row.
        assert!(h.console("share_join", json!({"share": id})).is_err());
    }

    /// §8: a guest reads only a session shared with them and only once let
    /// in: not another guest's, not the other session in the same place,
    /// not the later session after `/clear` until Share again; a removed
    /// guest reads nothing.
    #[test]
    fn a_guest_reads_only_the_session_shared_with_them() {
        let h = Host::new("hist-who");
        let (id, old) = shared(&h, "s-old", &(user("u1", "first") + &reply("ok")));
        let bret = Host::principal(&h.bret);
        let eve = Host::principal(&h.eve);
        // Invited only: nothing until allowed.
        h.console("share_remove_guest", json!({"share": id, "principal": eve})).unwrap();
        let other = h.top.join("s-eve.jsonl");
        write(&other, &user("e1", "eve's place"));
        h.live("other", "s-eve", 0);
        let eves = h.console("share_create", json!({"repo": h.a, "slug": "other", "guests": [eve]})).unwrap()["id"].as_str().unwrap().to_string();
        assert!(h.run(&h.eve, "shared_turns", json!({"share": eves})).unwrap_err().as_str().unwrap().contains("not let in yet"));
        // Another guest's share is missing to Bret.
        assert_eq!(h.run(&h.bret, "shared_turns", json!({"share": eves})), Err(json!(res::ERR_NOT_FOUND)));
        assert_eq!(h.run(&h.bret, "shared_tool_call", json!({"share": eves, "call_id": "x"})), Err(json!(res::ERR_NOT_FOUND)));
        assert_eq!(says(&h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap()), vec!["first"]);
        // `/clear` in the same place: a new id, its own file. The poll ends
        // the binding; the new session's turns never reach Bret.
        let new = h.top.join("s-new.jsonl");
        write(&new, &user("n1", "private after clear"));
        h.live("feat", "s-new", 0);
        h.poll();
        let v = h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap();
        assert_eq!((says(&v), v["state"].as_str()), (vec!["first".to_string()], Some("ended")));
        assert!(!v.to_string().contains("private after clear"));
        // Share again binds it; the ended session is `binding: 0`.
        h.console("share_again", json!({"share": id})).unwrap();
        assert_eq!(says(&h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap()), vec!["private after clear"]);
        assert_eq!(says(&h.run(&h.bret, "shared_turns", json!({"share": id, "binding": 0})).unwrap()), vec!["first"]);
        assert!(h.run(&h.bret, "shared_turns", json!({"share": id, "binding": 1})).is_err());
        // Removed: missing, at once.
        h.console("share_remove_guest", json!({"share": id, "principal": bret})).unwrap();
        assert_eq!(h.run(&h.bret, "shared_turns", json!({"share": id})), Err(json!(res::ERR_NOT_FOUND)));
        let _ = old;
    }

    /// §3.3: an ended binding serves nothing past its offset, even after its
    /// old id is resumed and the file grows — a later turn, a call in it, or
    /// a line joined to an earlier turn (a command's output).
    #[test]
    fn an_ended_binding_serves_nothing_past_its_offset() {
        let h = Host::new("hist-ended");
        let (id, old) = shared(&h, "s-old", &(user("u1", "before the end") + &call("c1", "ls") + &output("c1", "OUT")));
        h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap();
        h.live("feat", "s-new", 0);
        std::fs::write(h.top.join("s-new.jsonl"), "").unwrap();
        h.poll();
        // The old id resumed: it grows with a turn, a call, and a shell's
        // output that joins an earlier `!` turn.
        h.live("feat", "s-old", 0);
        append(&old, &(user("u2", "after the end") + &call("c2", "rm -rf secret")));
        h.poll();
        let v = h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap();
        assert_eq!(says(&v), vec!["before the end"]);
        assert!(!v.to_string().contains("after the end"), "{v}");
        assert_eq!(h.run(&h.bret, "shared_tool_call", json!({"share": id, "call_id": "c1"})).unwrap()["input"], json!("ls"));
        let gone = h.run(&h.bret, "shared_tool_call", json!({"share": id, "call_id": "c2"})).unwrap_err();
        assert!(gone.as_str().unwrap().contains("not found"), "{gone}");
        // Share again binds the resumed id afresh: the new binding reads it
        // all, the ended one still stops.
        h.console("share_again", json!({"share": id})).unwrap();
        assert_eq!(says(&h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap()), vec!["after the end", "before the end"]);
        assert_eq!(says(&h.run(&h.bret, "shared_turns", json!({"share": id, "binding": 0})).unwrap()), vec!["before the end"]);
    }

    /// The offset also cuts a line the index joins to an EARLIER turn: a
    /// shell's output written after the end is not read into the turn.
    #[test]
    fn an_ended_binding_cuts_output_joined_to_an_earlier_turn() {
        let shell = json!({"type": "user", "uuid": "s1", "timestamp": "2026-11-12T10:00:00Z", "message": {"role": "user", "content": "<bash-input>cat key</bash-input>"}}).to_string() + "\n";
        let out = json!({"type": "user", "uuid": "s2", "timestamp": "2026-11-12T10:00:00Z", "message": {"role": "user", "content": "<bash-stdout>SECRET-OUTPUT</bash-stdout><bash-stderr></bash-stderr>"}}).to_string() + "\n";
        let dir = std::env::temp_dir().join(format!("wt-hist-join-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("t.jsonl");
        let head = shell.clone() + &user("u1", "next");
        write(&t, &(head.clone() + &out));
        let b = Binding { harness: "claude".into(), session: "x".into(), transcript: t.clone(), bound_at: 0, state: BindingState::Ended, ended_offset: Some(head.len() as u64), ended_at: Some(1) };
        // The live read first, so the shared index runs past the offset and
        // has joined the output to the `!` turn.
        let mut live = b.clone();
        live.state = BindingState::Active;
        assert!(page(&dir, &live, None, None).unwrap().to_string().contains("SECRET-OUTPUT"), "the output does join, read whole");
        let v = page(&dir, &b, None, None).unwrap();
        assert!(!v.to_string().contains("SECRET-OUTPUT"), "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An ended session whose old id resumed and went on with the SAME turn:
    /// read after the index has run past the end (a live binding of the same
    /// file), the turn keeps what it had and gains nothing.
    #[test]
    fn an_ended_turn_continued_after_the_end_keeps_only_its_part() {
        let dir = std::env::temp_dir().join(format!("wt-hist-cont-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("t.jsonl");
        let head = user("u1", "the turn") + &reply("said before the end");
        write(&t, &(head.clone() + &reply("SAID AFTER THE END") + &call("c9", "late")));
        let ended = Binding { harness: "claude".into(), session: "x".into(), transcript: t.clone(), bound_at: 0, state: BindingState::Ended, ended_offset: Some(head.len() as u64), ended_at: Some(1) };
        let mut live = ended.clone();
        live.state = BindingState::Active;
        assert!(page(&dir, &live, None, None).unwrap().to_string().contains("SAID AFTER THE END"));
        let v = page(&dir, &ended, None, None).unwrap();
        assert_eq!(says(&v), vec!["the turn"]);
        assert!(v.to_string().contains("said before the end") && !v.to_string().contains("SAID AFTER"), "{v}");
        assert!(tool_call(&dir, &ended, "c9").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §8: `api-request-blob` content — the system prompt, every CLAUDE.md —
    /// never appears, nor tool output.
    #[test]
    fn api_request_blobs_and_tool_output_never_appear() {
        let h = Host::new("hist-blob");
        let text = blob("CLAUDE-MD-SECRET", 10) + &user("u1", "hi") + &blob("SYSTEM-PROMPT-SECRET", 10) + &call("c1", "cat .env") + &output("c1", "TOOL-OUTPUT-SECRET") + &reply("done");
        let (id, _) = shared(&h, "s-1", &text);
        let v = h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap();
        let call = h.run(&h.bret, "shared_tool_call", json!({"share": id, "call_id": "c1"})).unwrap();
        for s in [v.to_string(), call.to_string()] {
            for m in ["CLAUDE-MD-SECRET", "SYSTEM-PROMPT-SECRET", "TOOL-OUTPUT-SECRET", "api-request-blob", "/Users", h.top.to_str().unwrap()] {
                assert!(!s.contains(m), "{m} reached a guest: {s}");
            }
        }
        assert_eq!(says(&v), vec!["hi"]);
        assert_eq!(call["input"], json!("cat .env"));
    }

    /// The page's byte bound and the whole-turn cut, each said.
    #[test]
    fn a_page_keeps_its_byte_bound_and_a_cut_turn_says_so() {
        let h = Host::new("hist-bytes");
        // One turn over the per-turn cap.
        let long = "a".repeat(turns::TURN_TEXT_MAX + 5_000);
        // Thirty turns at the cap, ~100 KB each: over a page's 2 MiB.
        let mut text = user("u0", "the long one") + &reply(&long);
        for i in 1..=30 {
            text += &(user(&format!("u{i}"), &format!("turn {i}")) + &reply(&"b".repeat(turns::TURN_TEXT_MAX)));
        }
        let (id, _) = shared(&h, "s-1", &text);
        let v = h.run(&h.bret, "shared_turns", json!({"share": id, "limit": 50})).unwrap();
        let got = says(&v);
        assert!(serde_json::to_vec(&v).unwrap().len() <= PAGE_BYTES + 4096, "{} bytes", serde_json::to_vec(&v).unwrap().len());
        assert!(got.len() < 31 && v["page_full"] == json!(true) && v["more"] == json!(true), "{got:?}");
        assert_eq!(got[0], "turn 30", "newest first");
        // Paging back by the cursor reaches the long turn, cut and saying so.
        let mut before = v["next_before"].as_u64().unwrap();
        let mut last = v;
        while last["more"] == json!(true) {
            last = h.run(&h.bret, "shared_turns", json!({"share": id, "before": before, "limit": 50})).unwrap();
            before = last["next_before"].as_u64().unwrap();
        }
        let t = last["turns"].as_array().unwrap().last().unwrap().clone();
        assert_eq!((t["user"].as_str(), t["seq"].as_u64(), t["truncated"].as_bool()), (Some("the long one"), Some(0), Some(true)));
        assert!(t["chars"].as_u64().unwrap() > t["kept"].as_u64().unwrap(), "{}", t["chars"]);
        // The prompt, the reply to the cap, and the `…` that marks the cut.
        assert_eq!(t["kept"].as_u64(), Some(("the long one".len() + turns::TURN_TEXT_MAX + 1) as u64));
    }

    /// The index is read once and extended past what it read: a new read
    /// reads only the bytes added, and none when nothing was.
    #[test]
    fn the_index_extends_without_reading_again() {
        let h = Host::new("hist-extend");
        let (id, t) = shared(&h, "s-1", &(blob("B", 50_000) + &user("u1", "one") + &reply("r")));
        let len = |p: &Path| std::fs::metadata(p).unwrap().len();
        h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap();
        let first = INDEXES.index_bytes(&h.cfg, "claude", &t).unwrap();
        assert_eq!(first, len(&t));
        h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap();
        assert_eq!(INDEXES.index_bytes(&h.cfg, "claude", &t), Some(first), "a poll with nothing new reads nothing");
        let more = user("u2", "two");
        append(&t, &more);
        let v = h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap();
        assert_eq!(says(&v), vec!["two", "one"]);
        assert_eq!(INDEXES.index_bytes(&h.cfg, "claude", &t), Some(first + more.len() as u64), "only the new bytes");
        // Half a line: read again whole next time, never half-indexed.
        let line = user("u3", "three");
        append(&t, &line[..10]);
        assert_eq!(says(&h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap()).len(), 2);
        append(&t, &line[10..]);
        assert_eq!(says(&h.run(&h.bret, "shared_turns", json!({"share": id})).unwrap())[0], "three");
    }

    /// §8: a large transcript pages back to its OLDEST turn, and quickly
    /// once indexed: the oldest page reads that turn's bytes, not the file.
    #[test]
    fn a_large_transcript_pages_from_its_oldest_turn() {
        let h = Host::new("hist-large");
        let t = h.top.join("s-big.jsonl");
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&t).unwrap());
            let b = blob("BLOB", 100_000);
            for i in 0..400 {
                f.write_all(user(&format!("u{i}"), &format!("turn {i}")).as_bytes()).unwrap();
                f.write_all(b.as_bytes()).unwrap();
                f.write_all(call(&format!("c{i}"), &format!("echo {i}")).as_bytes()).unwrap();
                f.write_all(output(&format!("c{i}"), &"o".repeat(20_000)).as_bytes()).unwrap();
            }
        }
        let size = std::fs::metadata(&t).unwrap().len();
        assert!(size > 40_000_000, "{size}");
        let (id, _) = shared(&h, "s-small", "");
        let _ = id;
        // `shared` wrote an empty file under its own name; bind the big one.
        TEST_LIVE.lock().unwrap().insert(place_dir(&h.a, "other"), LiveSession::Running { harness: "claude", id: "s-big".into(), transcript: t.clone() });
        let bret = Host::principal(&h.bret);
        let big = h.console("share_create", json!({"repo": h.a, "slug": "other", "guests": [bret]})).unwrap()["id"].as_str().unwrap().to_string();
        h.console("share_allow", json!({"share": big, "principal": bret})).unwrap();
        let t0 = std::time::Instant::now();
        let v = h.run(&h.bret, "shared_turns", json!({"share": big})).unwrap();
        let build = t0.elapsed();
        assert_eq!(v["total"], json!(400));
        let t1 = std::time::Instant::now();
        let oldest = h.run(&h.bret, "shared_turns", json!({"share": big, "before": 1})).unwrap();
        let page = t1.elapsed();
        assert_eq!(says(&oldest), vec!["turn 0"]);
        assert_eq!(oldest["more"], json!(false));
        assert_eq!(INDEXES.index_bytes(&h.cfg, "claude", &t), Some(size), "a page reads its turn, not the file again");
        let c = h.run(&h.bret, "shared_tool_call", json!({"share": big, "call_id": "c0"})).unwrap();
        assert_eq!(c["input"], json!("echo 0"));
        eprintln!("{size} bytes: first read {build:?}, oldest page {page:?}");
        assert!(page < std::time::Duration::from_secs(2), "{page:?}");
        assert!(build < std::time::Duration::from_secs(60), "{build:?}");
    }

    /// §10's question, measured: a synthetic transcript of a few hundred MB,
    /// shaped like Claude's (a 100 KB `api-request-blob` per turn, five calls
    /// with 5 KB of output each, short replies). Run in release:
    /// `cargo test --release -p worktrees-cli --bin worktrees
    /// serve::history::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_the_index_on_a_large_transcript() {
        let dir = std::env::temp_dir().join(format!("wt-hist-measure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("big.jsonl");
        let turns_n: usize = std::env::var("WT_MEASURE_TURNS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&t).unwrap());
            let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
            let (blob_b, out_b) = (env("WT_MEASURE_BLOB", 100_000), env("WT_MEASURE_OUT", 5_000));
            let b = if blob_b == 0 { String::new() } else { blob("SYSTEM", blob_b) };
            for i in 0..turns_n {
                f.write_all(user(&format!("u{i}"), &format!("turn {i}: {}", "please do the thing ".repeat(10))).as_bytes()).unwrap();
                f.write_all(b.as_bytes()).unwrap();
                for c in 0..5 {
                    let id = format!("c{i}-{c}");
                    f.write_all(call(&id, &format!("cargo test -p worktrees-core {i} {c}")).as_bytes()).unwrap();
                    f.write_all(output(&id, &"o".repeat(out_b)).as_bytes()).unwrap();
                    f.write_all(reply(&"Looked at the result; next step. ".repeat(20)).as_bytes()).unwrap();
                }
            }
        }
        let size = std::fs::metadata(&t).unwrap().len();
        let rss = || {
            let o = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().unwrap();
            String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().unwrap_or(0) / 1024
        };
        let before = rss();
        let b = Binding { harness: "claude".into(), session: "x".into(), transcript: t.clone(), bound_at: 0, state: BindingState::Active, ended_offset: None, ended_at: None };
        let t0 = std::time::Instant::now();
        let v = page(&dir, &b, None, None).unwrap();
        let first = t0.elapsed();
        let after = rss();
        let t1 = std::time::Instant::now();
        page(&dir, &b, None, None).unwrap();
        let again = t1.elapsed();
        let t2 = std::time::Instant::now();
        let oldest = page(&dir, &b, Some(1), None).unwrap();
        let old_page = t2.elapsed();
        let mut grow = String::new();
        for i in 0..8 {
            grow += &(user(&format!("g{i}"), "more") + &blob("SYSTEM", 100_000) + &reply("ok"));
        }
        append(&t, &grow);
        let read0 = INDEXES.index_bytes(&dir, "claude", &t).unwrap();
        let t3 = std::time::Instant::now();
        let grown = page(&dir, &b, None, None).unwrap();
        let ext = t3.elapsed();
        let read1 = INDEXES.index_bytes(&dir, "claude", &t).unwrap();
        println!(
            "MEASURE {} MB, {} turns: first build+page {:?} ({:.0} MB/s), RSS {} -> {} MB; next poll {:?}; oldest page {:?}; +{} KB grown: page {:?}, read {} KB more; page json {} KB",
            size / 1_000_000, v["total"], first, size as f64 / 1e6 / first.as_secs_f64(), before, after, again, old_page, grow.len() / 1000, ext, (read1 - read0) / 1000,
            serde_json::to_vec(&v).unwrap().len() / 1000,
        );
        assert_eq!(oldest["turns"][0]["seq"], json!(0));
        assert_eq!(grown["total"].as_u64(), Some(turns_n as u64 + 8));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Codex rollout reads the same way, by its own parser.
    #[test]
    fn a_codex_rollout_pages_and_finds_its_calls() {
        let fx = include_str!("../../../worktrees-core/tests/fixtures/transcripts/codex-0.160.1-tools.jsonl");
        let dir = std::env::temp_dir().join(format!("wt-hist-codex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("rollout.jsonl");
        write(&t, fx);
        let b = Binding { harness: "codex".into(), session: "x".into(), transcript: t.clone(), bound_at: 0, state: BindingState::Active, ended_offset: None, ended_at: None };
        let v = page(&dir, &b, None, None).unwrap();
        let lines: Vec<String> = fx.lines().map(str::to_string).collect();
        let whole = turns::codex(&lines, turns::MAX_TURNS);
        assert_eq!(v["total"].as_u64(), Some(whole.len() as u64));
        let id = whole.iter().flat_map(|t| &t.steps).find_map(|s| match s {
            turns::Step::Tool { id, detail: true, .. } => Some(id.clone()),
            _ => None,
        });
        if let Some(id) = id {
            assert_eq!(tool_call(&dir, &b, &id).unwrap()["call_id"], json!(id));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
