//! The idea-session rows (`docs/proposals/idea-board-phases.md` §7.3,
//! phase S): conversations with Claude about one idea, and the chat that
//! drives them.
//!
//! Every row is addressed by the idea's id and, past the list and the start,
//! a conversation id from that idea's record (`worktrees_core::ideasessions`).
//! Serve declares them `HostConsole` on the objects `idea(id)` and
//! `idea_conv(conv, id)`: ideas are desktop only, so no device grant reaches
//! them.
//!
//! **The chat rows reuse the agent rows' bodies** (`agents::*_at`) on a
//! [`Locus::Idea`]: the same screen reads, proofs and paste, the same
//! `typed_line_ok` and empty-composer and no-prompt checks for `idea_send`
//! and `idea_answer_text` — `agent_send`'s path with a second, console-only
//! door (C14) — and one lock per conversation where a place has the place
//! lock. No slash commands and no attachments from this chat (phase S).
//!
//! **An ended conversation still reads**: its transcript is found by the
//! session id the record chose before the launch, so `idea_turns` answers
//! with `ended: true` (or says the transcript is no longer on disk).

use serde::Serialize;
use serde_json::{json, Value};
use worktrees_core::ideasessions::{self as sessions, Conversation, Start};
use worktrees_core::{activity, tmux};

use crate::agents::{self, Locus};
use crate::Ctx;

/// The conversation `conv` of the idea `id`, as a locus the agent bodies run
/// on.
fn locus(ctx: &Ctx, id: &str, conv: &str) -> Result<Locus, String> {
    let c = sessions::find_at(ctx.ideas_dir(), id, conv)?;
    Ok(Locus::Idea { session: sessions::session_name(&c.id), dir: ctx.ideas_dir().to_path_buf(), session_id: c.session_id })
}

/// One conversation as the list shows it.
#[derive(Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub conversation: Conversation,
    /// Its tmux session.
    pub session: String,
    /// `running` (its session is up), `busy`, `waiting` (on the person) or
    /// `ended` (no session: closed, or its Claude exited).
    pub state: &'static str,
}

fn state_of(live: bool, a: &activity::Activity) -> &'static str {
    match (live, a.state) {
        (false, _) => "ended",
        (true, activity::State::Busy) => "busy",
        (true, activity::State::Waiting) => "waiting",
        (true, _) => "running",
    }
}

/// `idea_conversations`: the idea's conversations, newest first, each with
/// its state.
pub fn idea_conversations(ctx: &Ctx, id: String) -> Result<Value, String> {
    let list = sessions::list_at(ctx.ideas_dir(), &id)?;
    let live = tmux::session_names();
    let io = ctx.agents.io();
    let rows: Vec<Listed> = list
        .into_iter()
        .map(|c| {
            let session = sessions::session_name(&c.id);
            let up = live.contains(&session);
            let a = if up {
                io.activity(&Locus::Idea { session: session.clone(), dir: ctx.ideas_dir().to_path_buf(), session_id: c.session_id.clone() })
            } else {
                activity::Activity::none()
            };
            Listed { state: state_of(up, &a), session, conversation: c }
        })
        .collect();
    Ok(json!({ "id": id, "conversations": rows }))
}

/// `idea_conversation_start`: a new conversation on the idea — the history
/// copy, `start.md`, the record (with the session id) and then the launch.
/// A refused launch (the live-agent limit, a Claude below the floor) leaves
/// nothing behind but the history copy.
pub fn idea_conversation_start(ctx: &Ctx, id: String) -> Result<Value, String> {
    let (c, session) = sessions::start_at(ctx.ideas_dir(), &id)?;
    Ok(json!({ "id": id, "conversation": c, "session": session }))
}

/// `idea_conversation_resume`: launch an ended conversation again,
/// `--resume '<its session id>'` under the same tmux name, with no opener.
pub fn idea_conversation_resume(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    let session = sessions::launch_at(ctx.ideas_dir(), &id, &conv, Start::Resume)?;
    Ok(json!({ "id": id, "conv": conv, "session": session }))
}

/// `idea_conversation_close`: end its tmux session. The record and Claude's
/// transcript stay.
pub fn idea_conversation_close(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    sessions::find_at(ctx.ideas_dir(), &id, &conv)?;
    let was = sessions::close(&conv)?;
    Ok(json!({ "id": id, "conv": conv, "closed": was }))
}

/// `idea_conversation_remove`: close it if it runs, then forget it — its
/// record entry and its folder. Claude's transcript is Claude's file.
pub fn idea_conversation_remove(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    sessions::find_at(ctx.ideas_dir(), &id, &conv)?;
    sessions::close(&conv)?;
    sessions::remove_at(ctx.ideas_dir(), &id, &conv)?;
    Ok(json!({ "id": id, "conv": conv, "removed": true }))
}

/// `idea_conversation_rename`: a new title, one line.
pub fn idea_conversation_rename(ctx: &Ctx, id: String, conv: String, title: String) -> Result<Value, String> {
    let c = sessions::rename_at(ctx.ideas_dir(), &id, &conv, &title)?;
    Ok(json!({ "id": id, "conversation": c }))
}

/// Close every live conversation of idea `id`: what removing the idea does
/// first, so no Claude keeps talking about a file that is gone.
pub fn close_all(ctx: &Ctx, id: &str) {
    if let Ok(list) = sessions::list_at(ctx.ideas_dir(), id) {
        for c in list {
            if let Err(e) = sessions::close(&c.id) {
                crate::log::warn(&format!("idea {id}: could not close conversation {}: {e}", c.id));
            }
        }
    }
}

// ── the chat: the agent rows' bodies on the conversation ────────────────────

pub fn idea_turns(ctx: &Ctx, id: String, conv: String, n: Option<u32>) -> Result<Value, String> {
    agents::turns_at(ctx, &locus(ctx, &id, &conv)?, None, n)
}

pub fn idea_turn_text(ctx: &Ctx, id: String, conv: String, turn_id: String) -> Result<Value, String> {
    agents::turn_text_at(ctx, &locus(ctx, &id, &conv)?, turn_id)
}

pub fn idea_tool_call(ctx: &Ctx, id: String, conv: String, call_id: String) -> Result<Value, String> {
    agents::tool_call_at(ctx, &locus(ctx, &id, &conv)?, call_id)
}

pub fn idea_prompt(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    agents::prompt_at(ctx, &locus(ctx, &id, &conv)?)
}

pub fn idea_answer(ctx: &Ctx, id: String, conv: String, choice: u32, fingerprint: String) -> Result<Value, String> {
    agents::answer_at(ctx, &locus(ctx, &id, &conv)?, choice, fingerprint)
}

pub fn idea_answer_text(ctx: &Ctx, id: String, conv: String, choice: u32, fingerprint: String, text: String) -> Result<Value, String> {
    agents::answer_text_at(ctx, &locus(ctx, &id, &conv)?, choice, fingerprint, text)
}

pub fn idea_answer_many(ctx: &Ctx, id: String, conv: String, choices: Vec<u32>, fingerprint: String) -> Result<Value, String> {
    agents::answer_many_at(ctx, &locus(ctx, &id, &conv)?, choices, fingerprint)
}

/// `idea_send`: `agent_send`'s body, into the conversation's own session.
pub fn idea_send(ctx: &Ctx, id: String, conv: String, text: String) -> Result<Value, String> {
    let at = locus(ctx, &id, &conv)?;
    let Locus::Idea { session, .. } = &at else { unreachable!() };
    let session = session.clone();
    agents::send_at(ctx, &at, session, text)
}

pub fn idea_interrupt(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    agents::interrupt_at(ctx, &locus(ctx, &id, &conv)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot's agent rows (the nav's dots and agent cell,
    /// `list_places`, the agent rows' lanes) find a place's agents through
    /// `agent_session_names`: an idea conversation is never one of them,
    /// even with its pane inside the place.
    #[test]
    fn a_places_agent_rows_never_list_an_idea_session() {
        let idea = sessions::session_name(&worktrees_core::ulid::new());
        let panes = tmux::PaneList::from_panes(vec![tmux::Pane {
            session: idea.clone(),
            path: "/r/repo".into(),
            cmd: "claude".into(),
            id: Some("%3".into()),
            ..tmux::Pane::default()
        }]);
        let rows = crate::workspace::agent_session_names(Some(&panes), "repo-(main)", "repo-(main)", false, "/r/repo", Some("/r/repo/.worktrees"));
        assert!(rows.iter().all(|(_, name, up)| !up && *name != idea), "{:?}", rows.iter().map(|r| (&r.1, r.2)).collect::<Vec<_>>());
    }

    /// No lanes and a transcript that reads, or none on disk.
    struct Ended(Option<Vec<String>>);

    impl agents::AgentIo for Ended {
        fn lanes(&self, _: &Locus) -> Vec<agents::Lane> {
            Vec::new()
        }
        fn activity(&self, _: &Locus) -> activity::Activity {
            activity::Activity::none()
        }
        fn capture(&self, _: &agents::Lane) -> Option<String> {
            panic!("an ended conversation has no screen to read")
        }
        fn press(&self, _: &agents::Lane, _: tmux::Key) -> Result<(), String> {
            panic!("no key reaches an ended conversation")
        }
        fn paste(&self, _: &agents::Lane, _: &str) -> Result<(), String> {
            panic!("no text reaches an ended conversation")
        }
        fn deliver(&self, _: &Locus, _: &agents::Lane, _: &str) -> Result<worktrees_core::harness::SendOutcome, String> {
            panic!("no text reaches an ended conversation")
        }
        fn transcript(&self, at: &Locus, lane: &agents::Lane) -> Result<Vec<String>, String> {
            assert!(matches!(at, Locus::Idea { .. }) && lane.pane.is_empty(), "read by id, through no pane");
            self.0.clone().ok_or_else(|| agents::ERR_TRANSCRIPT_GONE.to_string())
        }
        fn asked(&self, _: &agents::Lane) -> Vec<worktrees_core::pending::Asked> {
            Vec::new()
        }
        fn forget(&self, _: &agents::Lane, _: worktrees_core::pending::Forget) {}
        fn plan_file(&self, _: &str) -> Option<(String, String)> {
            None
        }
        fn codex_models(&self) -> Vec<(String, usize)> {
            Vec::new()
        }
        fn commands_of(&self, _: &Locus, _: &agents::Lane) -> (Option<String>, Vec<worktrees_core::slash::Command>) {
            (None, Vec::new())
        }
        fn command(&self, _: &agents::Lane, _: &str, _: &str) -> Result<worktrees_core::harness::SendOutcome, String> {
            panic!("no commands")
        }
        fn terminal_activity(&self, _: &agents::Lane) -> Result<u64, String> {
            Err("none".into())
        }
        fn sleep(&self, _: u64) {}
    }

    /// An ENDED conversation reads read-only from its transcript, found by
    /// the record's session id (`ended: true`); one Claude has since pruned
    /// says so in words. A send into it is refused before any pane is
    /// touched, and a place's chat with no lane is unchanged ("no agent").
    #[test]
    fn an_ended_conversation_reads_from_its_transcript_and_takes_no_text() {
        let dir = std::env::temp_dir().join(format!("wt-idea-ended-{}", worktrees_core::ulid::new()));
        let ideas = dir.join("ideas");
        let id = worktrees_core::ideas::create_at(&ideas, "Talk").unwrap().idea.id.unwrap();
        let conv = sessions::prepare_at(&ideas, &id).unwrap();
        let fixture = include_str!("../../worktrees-core/tests/fixtures/transcripts/claude-2.1.309.jsonl");
        let lines: Vec<String> = fixture.lines().map(str::to_string).collect();
        let ctx = |io: Ended| crate::Ctx::new(dir.join("cfg"), std::sync::Arc::new(crate::tests::Recorder::default())).with_agents(agents::Agents::new(std::sync::Arc::new(io))).with_ideas_dir(ideas.clone());
        let t = idea_turns(&ctx(Ended(Some(lines))), id.clone(), conv.id.clone(), None).unwrap();
        assert_eq!(t["ended"], json!(true), "{t}");
        assert!(!t["turns"].as_array().unwrap().is_empty(), "{t}");
        let gone = idea_turns(&ctx(Ended(None)), id.clone(), conv.id.clone(), None).unwrap();
        assert_eq!((gone["ended"].clone(), gone["reason"].clone()), (json!(true), json!(agents::ERR_TRANSCRIPT_GONE)));
        assert!(idea_send(&ctx(Ended(None)), id.clone(), conv.id.clone(), "hello".into()).is_err());
        assert!(idea_turns(&ctx(Ended(None)), id.clone(), worktrees_core::ulid::new(), None).is_err(), "a conversation the record lacks");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_conversation_reads_ended_without_a_session_and_its_activity_with_one() {
        let busy = activity::Activity { state: activity::State::Busy, ..activity::Activity::none() };
        let waiting = activity::Activity { state: activity::State::Waiting, ..activity::Activity::none() };
        assert_eq!(state_of(false, &busy), "ended");
        assert_eq!(state_of(true, &busy), "busy");
        assert_eq!(state_of(true, &waiting), "waiting");
        assert_eq!(state_of(true, &activity::Activity::none()), "running");
    }
}
