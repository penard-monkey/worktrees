//! An agent's last turns, read from the harness's own TRANSCRIPT — never its
//! pane — for serve's `agent_turns` row (iOS MVP §2.4, S9; F10, "recent
//! turns, not a terminal").
//!
//! A turn is one prompt and what the agent did with it: its reply text and
//! the names of the tools it called. Everything else in a transcript is left
//! where it is, and some of it must be. Claude's `.jsonl` carries
//! `api-request-blob` entries holding the whole system prompt and every
//! `CLAUDE.md` it loaded, `attachment`s, hook summaries and sidechains; Codex's
//! rollout opens on `session_meta.base_instructions` and `developer` messages,
//! and wraps AGENTS.md and the environment as `<…>` user messages. None of
//! that is a turn, so only plain user and assistant messages are read.
//! Thinking and reasoning are left out as well.
//!
//! Text is DATA. The reply is what the agent wrote, and what the user typed
//! may itself be a message another place sent; the row says so on its answer.

use serde::Serialize;

/// The most turns one answer carries.
pub const MAX_TURNS: usize = 20;
/// Characters kept of one prompt or one reply.
pub const TEXT_MAX: usize = 4000;
/// Bytes of turn text one answer carries, in all: the oldest turns are
/// dropped past it.
pub const TOTAL_MAX: usize = 64 * 1024;
/// How much of a transcript's end is read. Claude writes the system prompt
/// into the file on every request (`api-request-blob`, ~100 KB each), so the
/// window is wide; a turn that started before it is returned without its
/// prompt (`user: null`).
pub const TAIL_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// When its prompt was written (epoch seconds), from the entry's own
    /// timestamp — never the file's mtime.
    pub at: Option<i64>,
    /// The prompt. `None` when the window starts inside the turn.
    pub user: Option<String>,
    /// The agent's reply text, in order, blocks joined by a blank line.
    pub reply: String,
    /// The tools it called, by name, in order.
    pub tools: Vec<String>,
    /// A prompt or reply was cut at `TEXT_MAX`.
    pub truncated: bool,
}

impl Turn {
    fn new(at: Option<i64>, user: Option<String>) -> Turn {
        Turn { at, user, reply: String::new(), tools: Vec::new(), truncated: false }
    }

    fn say(&mut self, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            return;
        }
        if !self.reply.is_empty() {
            self.reply.push_str("\n\n");
        }
        self.reply.push_str(t);
    }

    fn bytes(&self) -> usize {
        self.user.as_ref().map_or(0, String::len) + self.reply.len() + self.tools.iter().map(String::len).sum::<usize>()
    }
}

/// The last `n` turns (at most [`MAX_TURNS`]) of a Claude transcript's `lines`,
/// oldest first. Only `user` and `assistant` entries off the main thread
/// count: a user entry whose content is text starts a turn, unless it is
/// claude's own bookkeeping (`isMeta`, a `<command-…>` / `<local-command-…>`
/// wrapper, an interruption marker); a tool result does not.
pub fn claude(lines: &[String], n: usize) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for l in lines {
        if !(l.contains("\"type\":\"user\"") || l.contains("\"type\":\"assistant\"")) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        if v.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) || v.get("isMeta").and_then(|s| s.as_bool()) == Some(true) {
            continue;
        }
        let at = v.get("timestamp").and_then(|t| t.as_str()).and_then(crate::sysclock::parse_iso8601);
        let content = v.pointer("/message/content");
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                let text = match content {
                    Some(serde_json::Value::String(s)) => Some(s.clone()),
                    Some(serde_json::Value::Array(blocks)) if !blocks.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result")) => {
                        let t: Vec<&str> = blocks.iter().filter_map(|b| b.get("text").and_then(|t| t.as_str())).collect();
                        (!t.is_empty()).then(|| t.join("\n"))
                    }
                    _ => None,
                };
                if let Some(text) = text.filter(|t| is_prompt(t)) {
                    turns.push(Turn::new(at, Some(text)));
                }
            }
            Some("assistant") => {
                let Some(serde_json::Value::Array(blocks)) = content else { continue };
                let turn = current(&mut turns, at);
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => turn.say(b.get("text").and_then(|t| t.as_str()).unwrap_or("")),
                        Some("tool_use") => {
                            if let Some(name) = b.get("name").and_then(|n| n.as_str()) {
                                turn.tools.push(name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    finish(turns, n)
}

/// A user text that is a prompt somebody wrote, not claude's own wrapper.
fn is_prompt(text: &str) -> bool {
    let t = text.trim_start();
    !t.is_empty()
        && !t.starts_with("<command-")
        && !t.starts_with("<local-command-")
        && !t.starts_with("<system-reminder>")
        && !t.starts_with("[Request interrupted")
}

/// The last `n` turns of a Codex rollout's `lines`: `response_item`
/// messages, a `user` one starting a turn unless it is one of the `<…>`-wrapped
/// blocks Codex injects (the environment, AGENTS.md, an image) or a
/// `# AGENTS.md instructions` block; an `assistant` one's `output_text` is
/// the reply, a `function_call` / `custom_tool_call` a tool. `developer` and
/// `system` messages, reasoning and `session_meta` are never read.
pub fn codex(lines: &[String], n: usize) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for l in lines {
        if !l.contains("\"response_item\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }
        let at = v.get("timestamp").and_then(|t| t.as_str()).and_then(crate::sysclock::parse_iso8601);
        let Some(p) = v.get("payload") else { continue };
        let texts = |kind: &str| -> Vec<String> {
            p.get("content")
                .and_then(|c| c.as_array())
                .map(|bs| {
                    bs.iter()
                        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some(kind))
                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        match (p.get("type").and_then(|t| t.as_str()), p.get("role").and_then(|r| r.as_str())) {
            (Some("message"), Some("user")) => {
                let typed: Vec<String> = texts("input_text").into_iter().filter(|t| codex_typed(t)).collect();
                if !typed.is_empty() {
                    turns.push(Turn::new(at, Some(typed.join("\n"))));
                }
            }
            (Some("message"), Some("assistant")) => {
                let turn = current(&mut turns, at);
                for t in texts("output_text") {
                    turn.say(&t);
                }
            }
            (Some("function_call" | "custom_tool_call"), _) => {
                if let Some(name) = p.get("name").and_then(|n| n.as_str()) {
                    current(&mut turns, at).tools.push(name.to_string());
                }
            }
            _ => {}
        }
    }
    finish(turns, n)
}

/// A user `input_text` block somebody typed, not one Codex wrapped in.
fn codex_typed(text: &str) -> bool {
    let t = text.trim_start();
    !t.is_empty() && !t.starts_with('<') && !t.starts_with("# AGENTS.md instructions")
}

/// The turn a reply belongs to: the last one, or a prompt-less one when the
/// window starts inside a turn.
fn current(turns: &mut Vec<Turn>, at: Option<i64>) -> &mut Turn {
    if turns.is_empty() {
        turns.push(Turn::new(at, None));
    }
    turns.last_mut().expect("just pushed")
}

/// The last `n` (at most `MAX_TURNS`), each text cut to `TEXT_MAX` characters.
fn finish(mut turns: Vec<Turn>, n: usize) -> Vec<Turn> {
    let keep = n.clamp(1, MAX_TURNS);
    let turns = turns.split_off(turns.len().saturating_sub(keep));
    turns
        .into_iter()
        .map(|mut t| {
            if let Some(u) = t.user.take() {
                let (u, cut) = clip(&u);
                t.user = Some(u);
                t.truncated |= cut;
            }
            let (r, cut) = clip(&t.reply);
            t.reply = r;
            t.truncated |= cut;
            t
        })
        .collect()
}

fn clip(s: &str) -> (String, bool) {
    if s.chars().count() <= TEXT_MAX {
        return (s.to_string(), false);
    }
    let mut out: String = s.chars().take(TEXT_MAX).collect();
    out.push('…');
    (out, true)
}

/// `turns` cut to [`TOTAL_MAX`] bytes of text, newest kept: the oldest go
/// first. The flag says whether any went. The newest turn always stays.
pub fn within_budget(mut turns: Vec<Turn>) -> (Vec<Turn>, bool) {
    let mut dropped = false;
    while turns.len() > 1 && turns.iter().map(Turn::bytes).sum::<usize>() > TOTAL_MAX {
        turns.remove(0);
        dropped = true;
    }
    (turns, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    /// A Claude transcript shaped like 2.1.309's (the entry types and fields a
    /// real session wrote, the content invented): the turns are the user's
    /// prompts and the replies, and NOTHING of the system prompt, the
    /// attachments, the hook summaries, a sidechain or claude's own command
    /// wrappers comes back.
    #[test]
    fn claude_turns_are_prompts_and_replies_only() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.309.jsonl"));
        let got = claude(&fx, 10);
        let all = serde_json::to_string(&got).unwrap();
        for secret in ["SYSTEM-PROMPT-MARKER", "CLAUDE-MD-MARKER", "ATTACHMENT-MARKER", "SIDECHAIN-MARKER", "THINKING-MARKER", "TOOL-OUTPUT-MARKER", "local-command"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        assert_eq!(got.len(), 3, "{got:#?}");
        assert_eq!(got[0].user.as_deref(), Some("Use the Bash tool to run exactly: touch probe-a.txt"));
        assert_eq!(got[0].tools, vec!["Bash".to_string()]);
        assert_eq!(got[0].reply, "Done.");
        assert_eq!(got[0].at, crate::sysclock::parse_iso8601("2026-10-23T18:48:01.000Z"));
        assert_eq!(got[1].tools, vec!["AskUserQuestion".to_string()]);
        assert_eq!(got[1].reply, "Blue");
        assert_eq!(got[2].reply, "", "a rejected tool call answers nothing");
        assert_eq!(claude(&fx, 1).len(), 1);
        assert_eq!(claude(&fx, 1)[0].user.as_deref(), got[2].user.as_deref(), "the LAST turns");
        // A window that starts mid-turn has a reply with no prompt.
        let tail: Vec<String> = fx.iter().skip_while(|l| !l.contains("\"Done.\"")).cloned().collect();
        let got = claude(&tail, 10);
        assert_eq!((got[0].user.as_deref(), got[0].reply.as_str()), (None, "Done."));
    }

    #[test]
    fn codex_turns_skip_what_codex_injects() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/codex-rollout.jsonl"));
        let got = codex(&fx, 10);
        let all = serde_json::to_string(&got).unwrap();
        for secret in ["BASE-INSTRUCTIONS-MARKER", "DEVELOPER-MARKER", "ENVIRONMENT-MARKER", "AGENTS-MD-MARKER", "REASONING-MARKER", "TOOL-OUTPUT-MARKER"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        assert_eq!(got.len(), 2, "{got:#?}");
        assert_eq!(got[0].user.as_deref(), Some("run the tests"));
        assert_eq!(got[0].tools, vec!["exec_command".to_string(), "apply_patch".to_string()]);
        assert_eq!(got[0].reply, "Running them.\n\nAll green.");
        assert_eq!(got[1].user.as_deref(), Some("push it"));
    }

    #[test]
    fn turns_are_capped_by_count_text_and_total_bytes() {
        let long = "x".repeat(TEXT_MAX + 10);
        let entry = |text: &str| serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}).to_string();
        let fx: Vec<String> = (0..30).map(|i| entry(&format!("{i} {long}"))).collect();
        let got = claude(&fx, 999);
        assert_eq!(got.len(), MAX_TURNS);
        assert!(got.iter().all(|t| t.truncated && t.user.as_ref().unwrap().chars().count() == TEXT_MAX + 1));
        assert!(got.last().unwrap().user.as_ref().unwrap().starts_with("29 "), "the newest are kept");
        assert_eq!(claude(&fx, 0).len(), 1, "n is at least one");
        let (kept, dropped) = within_budget(got);
        assert!(dropped && kept.iter().map(Turn::bytes).sum::<usize>() <= TOTAL_MAX);
        assert!(kept.last().unwrap().user.as_ref().unwrap().starts_with("29 "));
    }
}
