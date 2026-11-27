//! The engine wakes an idle workspace chat that has new messages
//! (`docs/proposals/workspace-chat-powers.md` §10, the owner's yes of
//! 2026-11-27). The one path that types into Claude with no person behind
//! the act, so it is as narrow as it can be:
//!
//! - ONE fixed line ([`line`]): a count and the senders' addresses, never a
//!   word of any message, which stays in the inbox for the `messages` tool.
//! - Typed through `agents::send_at`'s own body (`agents::wake_at`): the
//!   chat's lock, no prompt up, an empty composer, and on that same fresh
//!   capture the turn shown idle (`prompt::turn_shown`).
//! - Only for a live chat the poll sees idle (not busy, no prompt, no fold
//!   running), with the Workspace chats beta on ([`beta_on`]).
//! - Once per batch: a message the chat was already woken for never wakes
//!   it again. At most one try per chat per [`WAKE_GAP_MS`], typed or not,
//!   so a burst of reports is one line and a draft is not read every tick.
//! - Only a PLACE's message wakes a chat ([`wakes_for`]). Another chat's
//!   waits in the count and rides along with the next place's wake. The
//!   engine wakes nothing but chats, so a chain of wakes always ends: a chat
//!   that reports to another chat on waking cannot wake it.
//! - Each line typed is an audit record (serve's `Audit::wake`): the ids,
//!   the count, the line's length and hash. Nothing reaches a paired device.

use std::collections::{BTreeMap, HashMap, HashSet};

use worktrees_core::messages::{self, Message};
use worktrees_core::workspacechats::{self as wc, Address};

use crate::watch::WorkspaceSessions;
use crate::Ctx;

/// The least time between two tries at one chat, typed or refused.
pub const WAKE_GAP_MS: i64 = 60_000;
/// How many senders the line names before "and N more".
const NAMED: usize = 3;
/// The longest sender the line names as it is.
const SENDER_MAX: usize = 80;

/// What the poll remembers between ticks, by chat id. Memory only: a restart
/// may wake a chat once more for a batch it was woken for already.
#[derive(Default, Debug)]
pub struct Wakes {
    by_chat: HashMap<String, Seen>,
}

#[derive(Default, Debug)]
struct Seen {
    /// The unread messages a typed line already counted.
    told: HashSet<String>,
    /// When the last try was made, in ms.
    tried: Option<i64>,
}

/// One line the engine typed, for the host's audit log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeNote {
    pub workspace: String,
    pub chat: String,
    /// `woke`, or `stopped` when the paste did not finish.
    pub event: &'static str,
    /// How many unread messages the line counted.
    pub count: usize,
    /// The line itself: engine words, a count and addresses. The audit
    /// keeps only its length and hash.
    pub line: String,
}

/// Whether the Workspace chats beta is on: `beta_workspace_chats` in the
/// app's `ui-state.json`, READ only (the frontend owns that file whole).
pub fn beta_on(config_dir: &std::path::Path) -> bool {
    std::fs::read(config_dir.join("ui-state.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v.get("beta_workspace_chats") == Some(&serde_json::Value::Bool(true)))
}

/// Whether a message from `from` may wake a chat: a place's may, another
/// chat's may not ([`Address::claims`]).
pub fn wakes_for(from: &str) -> bool {
    !Address::claims(from)
}

/// A sender as the line names it: an address of plain characters, cut short;
/// anything else is "a place".
fn sender(from: &str) -> String {
    let ok = !from.is_empty() && from.chars().all(|c| c.is_ascii_alphanumeric() || "._:/()@~+-".contains(c));
    if !ok {
        return "a place".into();
    }
    if from.chars().count() > SENDER_MAX {
        return format!("{}…", from.chars().take(SENDER_MAX).collect::<String>());
    }
    from.to_string()
}

/// The one line: `[worktrees: 2 new messages for this chat, from a, b. Read
/// them with the messages tool. The engine typed this line; it needs no
/// reply.]` Senders in the order they first wrote.
pub fn line(count: usize, from: &[String]) -> String {
    let mut names: Vec<String> = Vec::new();
    for f in from.iter().map(|f| sender(f)) {
        if !names.contains(&f) {
            names.push(f);
        }
    }
    let more = names.len().saturating_sub(NAMED);
    names.truncate(NAMED);
    let mut who = names.join(", ");
    if more > 0 {
        who.push_str(&format!(" and {more} more"));
    }
    let what = if count == 1 { "1 new message".to_string() } else { format!("{count} new messages") };
    format!("{} {what} for this chat, from {who}. Read them with the messages tool. The engine typed this line; it needs no reply.]", wc::WAKE_PREFIX)
}

/// The poll's part, after `workspacechats::tick` has filled `now`: wake each
/// chat that may be woken, and answer the lines typed.
pub fn tick(ctx: &Ctx, now: &WorkspaceSessions, w: &mut Wakes, now_ms: i64) -> Vec<WakeNote> {
    if now.chat_of.is_empty() || !now.unread.keys().any(|s| now.chat_of.contains_key(s)) {
        w.by_chat.retain(|c, _| now.chat_of.values().any(|o| &o.chat == c));
        return Vec::new();
    }
    if !beta_on(&ctx.config_dir) {
        return Vec::new();
    }
    tick_with(ctx, now, w, now_ms, &|ws, chat, line| crate::workspacechats::workspace_wake(ctx, ws, chat, line))
}

type Typer<'a> = dyn Fn(&str, &str, String) -> Result<serde_json::Value, String> + 'a;

fn tick_with(ctx: &Ctx, now: &WorkspaceSessions, w: &mut Wakes, now_ms: i64, typer: &Typer) -> Vec<WakeNote> {
    w.by_chat.retain(|c, _| now.chat_of.values().any(|o| &o.chat == c));
    let mut notes = Vec::new();
    let sessions: BTreeMap<&String, _> = now.chat_of.iter().collect();
    for (session, of) in sessions {
        let seen = w.by_chat.entry(of.chat.clone()).or_default();
        let Ok(inbox) = wc::inbox_at(ctx.workspaces_dir(), &of.workspace) else { continue };
        let unread: Vec<Message> = messages::unread(&inbox, &of.chat, None, now_ms);
        // What the chat has read no longer counts as told.
        seen.told.retain(|id| unread.iter().any(|m| &m.id == id));
        if !unread.iter().any(|m| wakes_for(&m.from) && !seen.told.contains(&m.id)) {
            continue;
        }
        let folding = now.fold.get(session).is_some_and(|f| f.state != "failed") || ctx.folds.is_folding(&of.chat);
        if now.busy.contains(session) || now.waiting.contains(session) || folding {
            continue;
        }
        if seen.tried.is_some_and(|t| now_ms - t < WAKE_GAP_MS) {
            continue;
        }
        seen.tried = Some(now_ms);
        let from: Vec<String> = unread.iter().map(|m| m.from.clone()).collect();
        let text = line(unread.len(), &from);
        match typer(&of.workspace, &of.chat, text.clone()) {
            Ok(v) => {
                let stopped = v.get("stopped") == Some(&serde_json::Value::Bool(true));
                // A stopped paste may have reached the composer: the batch is
                // told either way, so the line is never typed twice.
                seen.told.extend(unread.iter().map(|m| m.id.clone()));
                notes.push(WakeNote {
                    workspace: of.workspace.clone(),
                    chat: of.chat.clone(),
                    event: if stopped { "stopped" } else { "woke" },
                    count: unread.len(),
                    line: text,
                });
            }
            Err(e) => crate::log::log("info", &format!("workspace {}: chat {} not woken: {e}", of.workspace, of.chat)),
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{self, Lane, Locus};
    use crate::watch::{ChatOf, FoldOf};
    use std::sync::{Arc, Mutex};
    use worktrees_core::harness::SendOutcome;
    use worktrees_core::workspacechats as chats;
    use worktrees_core::{activity, tmux};

    macro_rules! fx {
        ($name:literal) => {
            include_str!(concat!("../../worktrees-core/tests/fixtures/claude-screen/2.1.309/", $name))
        };
    }

    /// One chat's pane: a scripted screen, what was typed, and an optional
    /// screen to show from the first capture on (a turn that began after the
    /// poll looked).
    struct Pane {
        session: String,
        screen: Mutex<String>,
        typed: Mutex<Vec<String>>,
        captures: Mutex<usize>,
    }

    impl agents::AgentIo for Pane {
        fn lanes(&self, at: &Locus) -> Vec<Lane> {
            assert!(matches!(at, Locus::Workspace { .. }), "a chat's own locus");
            vec![Lane { harness: "claude", session: self.session.clone(), pane: "%1".into() }]
        }
        fn activity(&self, _: &Locus) -> activity::Activity {
            activity::Activity::none()
        }
        fn capture(&self, _: &Lane) -> Option<String> {
            *self.captures.lock().unwrap() += 1;
            Some(self.screen.lock().unwrap().clone())
        }
        fn press(&self, _: &Lane, k: tmux::Key) -> Result<(), String> {
            panic!("the wake presses no key: {k:?}")
        }
        fn paste(&self, _: &Lane, _: &str) -> Result<(), String> {
            panic!("the wake types through deliver only")
        }
        fn deliver(&self, _: &Locus, _: &Lane, text: &str) -> Result<SendOutcome, String> {
            self.typed.lock().unwrap().push(text.to_string());
            Ok(SendOutcome::Submitted)
        }
        fn transcript(&self, _: &Locus, _: &Lane) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
        fn asked(&self, _: &Lane) -> Vec<worktrees_core::pending::Asked> {
            Vec::new()
        }
        fn forget(&self, _: &Lane, _: worktrees_core::pending::Forget) {}
        fn plan_file(&self, _: &str) -> Option<(String, String)> {
            None
        }
        fn codex_models(&self) -> Vec<(String, usize)> {
            Vec::new()
        }
        fn commands_of(&self, _: &Locus, _: &Lane) -> (Option<String>, Vec<worktrees_core::slash::Command>) {
            (None, Vec::new())
        }
        fn command(&self, _: &Lane, _: &str, _: &str) -> Result<SendOutcome, String> {
            panic!("no commands")
        }
        fn terminal_activity(&self, _: &Lane) -> Result<u64, String> {
            Err("none".into())
        }
        fn sleep(&self, _: u64) {}
        fn session_up(&self, _: &str) -> bool {
            true
        }
    }

    struct World {
        top: std::path::PathBuf,
        dir: std::path::PathBuf,
        ws: String,
        chat: String,
        session: String,
        pane: Arc<Pane>,
        ctx: Ctx,
    }

    impl World {
        fn new(tag: &str, screen: &str, beta: bool) -> World {
            let top = std::env::temp_dir().join(format!("wt-wake-{tag}-{}", worktrees_core::ulid::new()));
            let dir = top.join("workspaces");
            let ws = worktrees_core::ulid::new();
            let c = chats::prepare_at(&dir, &ws, |_| String::new()).unwrap();
            let session = chats::session_name(&c.id);
            let pane = Arc::new(Pane { session: session.clone(), screen: Mutex::new(screen.into()), typed: Mutex::default(), captures: Mutex::default() });
            std::fs::create_dir_all(top.join("cfg")).unwrap();
            std::fs::write(top.join("cfg/ui-state.json"), serde_json::json!({ "beta_workspace_chats": beta }).to_string()).unwrap();
            let ctx = Ctx::new(top.join("cfg"), Arc::new(crate::tests::Recorder::default()))
                .with_agents(agents::Agents::new(pane.clone()))
                .with_workspaces_dir(dir.clone());
            World { top, dir, ws, chat: c.id, session, pane, ctx }
        }

        fn post(&self, from: &str, text: &str, at: i64) -> String {
            let inbox = wc::inbox_at(&self.dir, &self.ws).unwrap();
            messages::post(&inbox, from, &self.chat, text, None, None, at).unwrap().id
        }

        fn read(&self, ids: &[String]) {
            messages::ack(&wc::inbox_at(&self.dir, &self.ws).unwrap(), &self.chat, ids).unwrap();
        }

        /// The poll's view: the chat live and idle, its unread count filled.
        fn sessions(&self) -> WorkspaceSessions {
            let n = messages::unread(&wc::inbox_at(&self.dir, &self.ws).unwrap(), &self.chat, None, T0).len();
            WorkspaceSessions {
                live: vec![self.session.clone()],
                chat_of: [(self.session.clone(), ChatOf { workspace: self.ws.clone(), chat: self.chat.clone() })].into(),
                unread: if n > 0 { [(self.session.clone(), n)].into() } else { Default::default() },
                ..WorkspaceSessions::default()
            }
        }

        fn typed(&self) -> Vec<String> {
            self.pane.typed.lock().unwrap().clone()
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.top);
        }
    }

    const T0: i64 = 1_900_000_000_000;

    /// The line carries the count and the senders, never a message's words.
    #[test]
    fn the_line_is_a_count_and_the_senders_only() {
        let l = line(2, &["worktrees:kid".into(), "worktrees:kid".into()]);
        assert_eq!(l, "[worktrees: 2 new messages for this chat, from worktrees:kid. Read them with the messages tool. The engine typed this line; it needs no reply.]");
        assert_eq!(line(1, &["a".into()]).split(" for ").next(), Some("[worktrees: 1 new message"));
        let many = line(5, &["a".into(), "b".into(), "c".into(), "d".into(), "e\u{202e}x".into()]);
        assert!(many.contains("from a, b, c and 2 more."), "{many}");
        assert_eq!(sender("x\ny"), "a place");
        assert_eq!(sender(&"k".repeat(200)).chars().count(), SENDER_MAX + 1);
        assert!(worktrees_core::harness::typed_line_ok(&l).is_ok(), "the composer's own rule takes it");
        assert!(l.starts_with(wc::WAKE_PREFIX), "start.md names the line by this");
    }

    /// A place's report to an idle chat types the line once, and only once
    /// for that batch; a new report after the gap is a new batch. The line
    /// holds no word of the message.
    #[test]
    fn a_report_wakes_an_idle_chat_once_per_batch() {
        let w = World::new("once", fx!("composer-after-turn.e.txt"), true);
        let a = w.post("worktrees:kid", "SECRET-WORDS", T0);
        let mut wakes = Wakes::default();
        let notes = tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 1_000);
        assert_eq!(w.typed().len(), 1, "one line");
        assert!(!w.typed()[0].contains("SECRET"), "{}", w.typed()[0]);
        assert_eq!(notes, vec![WakeNote { workspace: w.ws.clone(), chat: w.chat.clone(), event: "woke", count: 1, line: w.typed()[0].clone() }]);
        // Unread still, but already told: never again for it.
        assert!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 120_000).is_empty());
        assert_eq!(w.typed().len(), 1);
        // A second batch after the gap: one more line, counting both.
        w.post("worktrees:kid2", "more", T0 + 121_000);
        let n = tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 122_000);
        assert_eq!((n.len(), n[0].count), (1, 2));
        assert!(w.typed()[1].contains("from worktrees:kid, worktrees:kid2."), "{}", w.typed()[1]);
        // Read, then nothing.
        w.read(&[a]);
        assert!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 400_000).is_empty());
    }

    /// Two batches a second apart are one line: the second waits for the
    /// gap, and then goes only if the chat has not read it.
    #[test]
    fn two_batches_a_second_apart_are_one_line_until_the_gap() {
        let w = World::new("burst", fx!("composer-after-turn.e.txt"), true);
        let mut wakes = Wakes::default();
        w.post("worktrees:a", "x", T0);
        assert_eq!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 100).len(), 1);
        let b = w.post("worktrees:b", "y", T0 + 1_000);
        assert!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 1_100).is_empty(), "inside the gap");
        assert!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + WAKE_GAP_MS).is_empty(), "still inside it");
        assert_eq!(w.typed().len(), 1);
        assert_eq!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 100 + WAKE_GAP_MS).len(), 1, "after it");
        assert_eq!(w.typed().len(), 2);
        // Had the chat read the second, nothing would be typed.
        let w2 = World::new("burst2", fx!("composer-after-turn.e.txt"), true);
        let mut wakes2 = Wakes::default();
        w2.post("worktrees:a", "x", T0);
        let _ = tick(&w2.ctx, &w2.sessions(), &mut wakes2, T0 + 100);
        let b2 = w2.post("worktrees:b", "y", T0 + 1_000);
        let first: Vec<String> = messages::unread(&wc::inbox_at(&w2.dir, &w2.ws).unwrap(), &w2.chat, None, T0 + 2_000).into_iter().map(|m| m.id).collect();
        w2.read(&first);
        assert!(tick(&w2.ctx, &w2.sessions(), &mut wakes2, T0 + 200_000).is_empty());
        assert_eq!(w2.typed().len(), 1);
        let _ = (b, b2);
    }

    /// A busy, waiting or folding chat is never typed into, and neither is a
    /// chat whose turn began after the poll looked: the fresh capture under
    /// the lock shows it running, and nothing is pasted.
    #[test]
    fn a_chat_that_is_not_idle_gets_nothing() {
        let w = World::new("busy", fx!("composer-after-turn.e.txt"), true);
        w.post("worktrees:kid", "x", T0);
        let mut wakes = Wakes::default();
        for (i, f) in [
            |s: &mut WorkspaceSessions, n: &str| s.busy.push(n.into()),
            |s: &mut WorkspaceSessions, n: &str| s.waiting.push(n.into()),
            |s: &mut WorkspaceSessions, n: &str| {
                s.fold.insert(n.into(), FoldOf { workspace: String::new(), chat: String::new(), state: "folding".into(), why: None, left: None });
            },
        ]
        .into_iter()
        .enumerate()
        {
            let mut s = w.sessions();
            f(&mut s, &w.session);
            assert!(tick(&w.ctx, &s, &mut wakes, T0 + 1_000 * (i as i64 + 1)).is_empty());
        }
        assert_eq!(*w.pane.captures.lock().unwrap(), 0, "the poll's own view decides first");
        // Idle by the poll, running on the screen: busy between check and paste.
        *w.pane.screen.lock().unwrap() = fx!("composer-busy-empty.e.txt").into();
        assert!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 10_000).is_empty());
        assert_eq!(*w.pane.captures.lock().unwrap(), 1, "looked, under the lock");
        // A draft, and a prompt, are refused the same way.
        for screen in [fx!("composer-typed.e.txt"), fx!("trust.txt")] {
            *w.pane.screen.lock().unwrap() = screen.into();
            let mut fresh = Wakes::default();
            assert!(tick(&w.ctx, &w.sessions(), &mut fresh, T0 + 20_000).is_empty());
        }
        assert!(w.typed().is_empty(), "{:?}", w.typed());
        // A refusal waits for the gap before the next look.
        *w.pane.screen.lock().unwrap() = fx!("composer-after-turn.e.txt").into();
        assert!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 11_000).is_empty(), "the try at 10 s counts");
        assert_eq!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 10_000 + WAKE_GAP_MS).len(), 1);
    }

    /// Another chat's message waits in the count and wakes nothing: a chat
    /// that reports to another chat on waking cannot wake it back. It rides
    /// along with the next place's wake.
    #[test]
    fn another_chats_message_wakes_nothing() {
        let w = World::new("chat", fx!("composer-after-turn.e.txt"), true);
        let other = Address::of(&w.ws, &worktrees_core::ulid::new()).text();
        w.post(&other, "ping", T0);
        let mut wakes = Wakes::default();
        assert!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 1_000).is_empty());
        assert!(w.typed().is_empty());
        w.post("worktrees:kid", "done", T0 + 2_000);
        let n = tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 3_000);
        assert_eq!(n.len(), 1);
        assert!(n[0].line.starts_with("[worktrees: 2 new messages") && n[0].line.contains(&other), "{}", n[0].line);
    }

    /// With the Workspace chats beta off, or no ui-state at all, nothing is
    /// typed.
    #[test]
    fn the_beta_off_wakes_nothing() {
        let w = World::new("beta", fx!("composer-after-turn.e.txt"), false);
        w.post("worktrees:kid", "x", T0);
        assert!(tick(&w.ctx, &w.sessions(), &mut Wakes::default(), T0 + 1_000).is_empty());
        std::fs::remove_file(w.top.join("cfg/ui-state.json")).unwrap();
        assert!(tick(&w.ctx, &w.sessions(), &mut Wakes::default(), T0 + 1_000).is_empty());
        assert!(w.typed().is_empty());
    }

    /// A chat whose Claude has not started (or sits at its folder-trust
    /// question with no lane) gets nothing, and the batch is tried again
    /// once it has.
    #[test]
    fn a_chat_not_started_gets_nothing_and_is_tried_again() {
        let w = World::new("start", fx!("composer-after-turn.e.txt"), true);
        w.post("worktrees:kid", "x", T0);
        let mut wakes = Wakes::default();
        let refused = tick_with(&w.ctx, &w.sessions(), &mut wakes, T0 + 1_000, &|_, _, _| Err(agents::ERR_NOT_STARTED.into()));
        assert!(refused.is_empty());
        assert_eq!(tick(&w.ctx, &w.sessions(), &mut wakes, T0 + 1_000 + WAKE_GAP_MS).len(), 1, "not told, so tried again");
    }
}
