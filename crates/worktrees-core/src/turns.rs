//! An agent's last turns, read from the harness's own TRANSCRIPT — never its
//! pane — for serve's `agent_turns` row (iOS MVP §2.4, S9; F10, "recent
//! turns, not a terminal").
//!
//! A turn is one prompt and what the agent did with it: its reply text and
//! the names of the tools it called. Everything else in a transcript is left
//! where it is, and some of it must be. Claude's `.jsonl` carries
//! `api-request-blob` entries holding the whole system prompt and every
//! `CLAUDE.md` it loaded, hook summaries and sidechains; Codex's
//! rollout opens on `session_meta.base_instructions` and `developer` messages,
//! and wraps AGENTS.md and the environment as `<…>` user messages. None of
//! that is a turn, so only user and assistant entries are considered, and
//! Claude's own user entries are classified before they reach a client. A
//! human `queued_command` attachment absorbed mid-turn is the exception: it is
//! a person's message inside that turn, not a new `user` entry. Its `rendered`
//! reminder is never read.
//! Thinking and reasoning are left out as well.
//!
//! Text is DATA. The reply is what the agent wrote, and what the user typed
//! may itself be a message another place sent; the row says so on its answer.
//!
//! **What a tool call was given stays off the turns** (a step says `detail`
//! when there is more than its line), and is read one call at a time:
//! [`claude_tool_call`], [`codex_tool_call`], serve's `agent_tool_call`. Files'
//! contents ([`PATH_ONLY`]) and output never leave through it.
//!
//! **A turn's `steps` are the same turn in order**, each with its own time:
//! the reply's blocks, the tools it called (each with what it did, in a line,
//! and whether its result says it failed — never the output itself), and
//! what a person was ASKED and
//! answered on the way — a question (`AskUserQuestion`), a plan
//! (`ExitPlanMode`), a tool call allowed or refused at its permission prompt.
//! They are read from the transcript, so they are the same whoever answered:
//! a client's card, the terminal or a phone. Claude 2.1.316 writes every one
//! of them (measured on a scratch session, `claude-2.1.316-answers.jsonl`):
//! the tool's `input` on the assistant's `tool_use`; on the user entry that
//! carries its `tool_result`, the decision (`permissionDecision.source` names
//! a person — `user_*` — or `config`, which asked nobody), a refusal
//! (`toolDenialKind`) and its note (`userFeedback`), a question's answers
//! (`toolUseResult.answers`), and a note given with a Yes, as a text block
//! beside the result. An older Claude that writes none of these gets `tool`
//! steps. Codex 0.160.1's rollout keeps a plan-mode question
//! (`request_user_input`) and its answer, notes included
//! (`codex-0.160.1-answers.jsonl`), but never the Yes or No of an approval —
//! only the escalated call and its output, and a No's "aborted by user" is
//! what an Esc writes too — so an approval stays a `tool`. `reply` and
//! `tools` are as they were, for older clients.

use serde::Serialize;
use std::collections::{HashMap, HashSet};

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
/// Bytes kept of one call's input ([`claude_tool_call`], [`codex_tool_call`]).
/// The longest measured across this machine's recent transcripts on
/// 2026-11-04: a 15,015-character Bash command, a 6,654-character MCP input.
pub const CALL_INPUT_MAX: usize = 64 * 1024;
/// Characters of one turn's text a whole-turn read ([`claude_turn`],
/// [`codex_turn`], serve's `agent_turn_text`) keeps: its prompt, each of its
/// other long strings, and its reply's `text` steps in all. Measured on
/// 2026-11-06 over the 4,574 turns of the 799 Claude transcripts this machine
/// touched in 30 days (main checkout's `~/.claude/projects`): reply text per
/// turn p50 403, p90 2,998, p99 7,485, p99.9 13,454, longest 20,640
/// characters, so the cap is ~5x the longest; prompts reach 404,000 (a pasted
/// log or a handover), and one of those is cut at the cap with `truncated`.
/// At most ~400 KB on the wire, far inside serve's 16 MiB message limit, and
/// one read of one turn, never polled, so a phone can afford it.
pub const TURN_TEXT_MAX: usize = 100_000;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// The turn's own id, the same on every read and while it grows, for
    /// `agent_turn_text`: Claude's `uuid` of the prompt's entry (of the first
    /// entry in the window, for a turn the window starts inside), Codex's
    /// `turn_id`. Absent from an older host, and from a turn that has none to
    /// give.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// When its prompt was written (epoch seconds), from the entry's own
    /// timestamp — never the file's mtime.
    pub at: Option<i64>,
    /// The prompt. `None` when the window starts inside the turn.
    pub user: Option<String>,
    /// Claude's transcript injected this entry; clients draw it outside the
    /// person's message bubble. Absent on older hosts and ordinary turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<Notice>,
    /// The agent's reply text, in order, blocks joined by a blank line.
    pub reply: String,
    /// The tools it called, by name, in order.
    pub tools: Vec<String>,
    /// A prompt or reply was cut at `TEXT_MAX`.
    pub truncated: bool,
    /// The turn in order: `reply`'s blocks, `tools`, and what was asked and
    /// answered between them, each step with its own time.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
}

/// A Claude-generated user entry, classified from flags, origin, or a whole
/// wrapper. All fields are display text: controls and invisible format
/// characters are removed, and long values are clipped.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Notice {
    Task { status: String, summary: String },
    PeerMessage { from_name: String, text: String },
    Command { name: String, args: String, #[serde(skip_serializing_if = "Option::is_none")] output: Option<String> },
    Shell { command: String, #[serde(skip_serializing_if = "Option::is_none")] output: Option<String>, #[serde(skip_serializing_if = "Option::is_none")] stderr: Option<String> },
    Interrupt,
    Summary { text: String },
}

/// One thing that happened in a turn.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Step {
    /// A block of the reply.
    Text { at: Option<i64>, text: String },
    /// A person's message Claude absorbed while working on this turn.
    User { at: Option<i64>, text: String },
    /// A tool it called that nobody was asked about.
    Tool {
        at: Option<i64>,
        name: String,
        /// The call's own id in the transcript (Claude's `tool_use` id,
        /// Codex's `call_id`), the same on every read. Absent from an older
        /// host.
        #[serde(rename = "call_id", skip_serializing_if = "String::is_empty")]
        id: String,
        /// What it did, in a line: a command, the file it read or wrote, a
        /// search's pattern ([`summary_of`], [`codex_summary`]). None for a
        /// tool whose input is not read: an MCP tool's, any unknown one's.
        /// Transcript text, so it is DATA, with the control and invisible
        /// characters removed. Older summaries go first when the answer
        /// is over [`TOTAL_MAX`] ([`within_budget`]).
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        /// Its result says it failed: Claude's `is_error`, Codex's failed
        /// command or script, or an interrupt. Only the flag: no output.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        failed: bool,
        /// What it was given. Never sent with the turns: that would spend the
        /// poll's budget on text nobody opened ([`CallDetail`] is how it leaves).
        #[serde(skip)]
        ran: Ran,
        /// `agent_tool_call` has more to say about this call than `summary`
        /// does. Absent from an older host, and for a [`PATH_ONLY`] tool.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        detail: bool,
    },
    /// `AskUserQuestion`: what it asked and, once given, the answer.
    Question {
        at: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        answered_at: Option<i64>,
        questions: Vec<Question>,
        #[serde(skip_serializing_if = "Option::is_none")]
        answer: Option<Answer>,
        #[serde(skip)]
        id: String,
    },
    /// A tool call a person allowed or refused at its permission prompt.
    Permission {
        at: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        answered_at: Option<i64>,
        tool: String,
        /// What it would do: a command's first line, a file's path.
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        allowed: bool,
        /// How long a Yes holds, as Claude wrote it (`once`, `session`).
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        /// What the person said with it: a No's reason, a Yes's next step.
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// `ExitPlanMode`: the plan and, once answered, approved or sent back.
    Plan {
        at: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        answered_at: Option<i64>,
        plan: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        plan_file: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        approved: Option<bool>,
        /// The feedback it was sent back with.
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(skip)]
        id: String,
    },
}

/// What a tool call was given, read when its step is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ran {
    /// Nothing to show: a [`PATH_ONLY`] tool, or an input that says nothing.
    Nothing,
    /// A command: a script's parts in order, `None` for a command it builds
    /// that no event has named.
    Parts(Vec<Option<String>>),
    /// Any other tool's input, an MCP tool's included.
    Json(serde_json::Value),
}

/// One question of an `AskUserQuestion`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// What the answer is keyed on, where it is not the question's text
    /// (Codex's question `id`).
    #[serde(skip)]
    pub key: String,
    pub header: String,
    pub question: String,
    pub multi: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// How a question was answered: `picked` (its options, and any text typed
/// as the answer), `chat` ("Chat about this"), or `declined` (refused any
/// other way, Esc).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub how: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<QuestionAnswer>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct QuestionAnswer {
    pub question: String,
    /// The options picked, by label.
    pub picked: Vec<String>,
    /// What was typed instead of, or beside, an option.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typed: Option<String>,
    /// A note given with the pick (Codex's `tab to add notes`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Turn {
    fn new(at: Option<i64>, user: Option<String>) -> Turn {
        Turn { id: String::new(), at, user, notice: None, reply: String::new(), tools: Vec::new(), truncated: false, steps: Vec::new() }
    }

    fn notice(at: Option<i64>, notice: Notice) -> Turn {
        // Keep a cleaned text fallback for clients that predate `notice`.
        let mut turn = Turn::new(at, Some(notice.fallback()));
        turn.notice = Some(notice);
        turn
    }

    fn say(&mut self, at: Option<i64>, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            return;
        }
        if !self.reply.is_empty() {
            self.reply.push_str("\n\n");
        }
        self.reply.push_str(t);
        self.steps.push(Step::Text { at, text: t.to_string() });
    }

    /// A tool call: its name in `tools`, and its step. A question and a plan
    /// are steps of their own from the start; any other call is a `tool`
    /// until its result says a person decided it.
    fn call(&mut self, at: Option<i64>, name: &str, id: &str, input: Option<&serde_json::Value>) {
        self.tools.push(name.to_string());
        let id = id.to_string();
        let str_of = |k: &str| input.and_then(|i| i.get(k)).and_then(|v| v.as_str()).map(str::to_string);
        self.steps.push(match name {
            "AskUserQuestion" => Step::Question { at, answered_at: None, questions: questions_of(input), answer: None, id },
            "ExitPlanMode" => Step::Plan { at, answered_at: None, plan: str_of("plan").unwrap_or_default(), plan_file: str_of("planFilePath"), approved: None, note: None, id },
            _ => Step::Tool { at, name: name.to_string(), id, summary: summary_of(name, input), failed: false, ran: ran_of(name, input), detail: false },
        });
    }

    fn bytes(&self) -> usize {
        self.id.len()
            + self.user.as_ref().map_or(0, String::len)
            + self.notice.as_ref().map_or(0, |n| serde_json::to_string(n).map_or(0, |s| s.len()))
            + self.reply.len()
            + self.tools.iter().map(String::len).sum::<usize>()
            + self.steps.iter().map(Step::bytes).sum::<usize>()
    }

    /// The part of [`Turn::bytes`] that is its tool calls' summaries.
    fn summary_bytes(&self) -> usize {
        self.steps.iter().map(|s| if let Step::Tool { summary: Some(s), .. } = s { s.len() } else { 0 }).sum()
    }
}

impl Notice {
    fn fallback(&self) -> String {
        match self {
            Notice::Task { status, summary } => format!("Background task {status}: {summary}"),
            Notice::PeerMessage { from_name, text } => format!("Message from {from_name}: {text}"),
            Notice::Command { name, args, output } => format!("/{name}{}{}{}", if args.is_empty() { "" } else { " " }, args, output.as_ref().map(|s| format!("\n{s}")).unwrap_or_default()),
            Notice::Shell { command, output, stderr } => format!("! {command}{}{}", output.as_ref().map(|s| format!("\n{s}")).unwrap_or_default(), stderr.as_ref().map(|s| format!("\n{s}")).unwrap_or_default()),
            Notice::Interrupt => "You stopped the turn".into(),
            Notice::Summary { text } => format!("Earlier conversation summarised\n{text}"),
        }
    }
}

/// The last `n` turns (at most [`MAX_TURNS`]) of a Claude transcript's `lines`,
/// oldest first. A `user` entry whose content is text starts a turn, unless it is
/// Claude's own injected entry (a structured notice, pure reminder, or local
/// command caveat) is classified or dropped; a tool result does not start a
/// turn. A human `queued_command` attachment belongs inside the current turn
/// only when its queue removal says `absorbed_mid_turn`. Other 2.1.316
/// consumption reasons are `delivered_to_agent`, `delivered_as_tool_result`
/// and `carried_to_background`; none is a mid-turn human message for this row.
pub fn claude(lines: &[String], n: usize) -> Vec<Turn> {
    finish(claude_turns(lines), n)
}

/// Every turn in a Claude transcript's `lines`, as [`claude`] reads them.
fn claude_turns(lines: &[String]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    struct Queued {
        turn: usize,
        step: usize,
        source: String,
        delivery: String,
        at: Option<i64>,
        text: String,
    }
    let mut queued: Vec<Queued> = Vec::new();
    // Claude may append the removal before the attachment it describes.
    // Match after scanning both, using their IDs rather than line order.
    let mut absorbed: HashMap<String, HashSet<String>> = HashMap::new();
    for l in lines {
        if !(l.contains("\"type\":\"user\"")
            || l.contains("\"type\":\"assistant\"")
            || l.contains("\"type\":\"attachment\"")
            || l.contains("\"type\":\"queue-operation\"")
            || l.contains("\"subtype\":\"local_command\""))
        {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        if v.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) {
            continue;
        }
        let at = v.get("timestamp").and_then(|t| t.as_str()).and_then(crate::sysclock::parse_iso8601);
        let content = v.pointer("/message/content");
        match v.get("type").and_then(|t| t.as_str()) {
            Some("attachment") => {
                let Some(a) = v.get("attachment") else { continue };
                let human = a.pointer("/origin/kind").and_then(|k| k.as_str()) == Some("human")
                    || a.get("humanTurn").and_then(|h| h.as_bool()) == Some(true);
                if !human || a.get("type").and_then(|t| t.as_str()) != Some("queued_command")
                    || a.get("commandMode").and_then(|m| m.as_str()) != Some("prompt")
                {
                    continue;
                }
                let (Some(source), Some(delivery), Some(raw), Some(turn)) = (
                    a.get("source_uuid").and_then(|s| s.as_str()),
                    a.get("delivery_id").and_then(|s| s.as_str()),
                    a.get("prompt").and_then(|s| s.as_str()),
                    turns.last(),
                ) else { continue };
                // A line break is the person's; everything else control goes.
                let text: String = pasted_body(raw).unwrap_or(raw).chars().filter(|c| *c == '\n' || (!c.is_control() && !crate::harness::invisible(*c))).collect();
                if !text.is_empty() {
                    queued.push(Queued { turn: turns.len() - 1, step: turn.steps.len(), source: source.to_string(), delivery: delivery.to_string(), at, text });
                }
            }
            Some("queue-operation") if v.get("operation").and_then(|o| o.as_str()) == Some("remove") => {
                if v.get("reason").and_then(|r| r.as_str()) != Some("absorbed_mid_turn") {
                    continue;
                }
                let (Some(source), Some(delivery)) = (
                    v.get("commandUuid").and_then(|s| s.as_str()),
                    v.get("deliveryId").and_then(|s| s.as_str()),
                ) else { continue };
                absorbed.entry(source.to_string()).or_default().insert(delivery.to_string());
            }
            Some("user") => {
                let text = match content {
                    Some(serde_json::Value::String(s)) => Some(s.clone()),
                    Some(serde_json::Value::Array(blocks)) if !blocks.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result")) => {
                        let t: Vec<&str> = blocks.iter().filter_map(|b| b.get("text").and_then(|t| t.as_str())).collect();
                        (!t.is_empty()).then(|| t.join("\n"))
                    }
                    _ => None,
                };
                if let Some(text) = text {
                    let before = turns.len();
                    absorb(&mut turns, at, claude_entry(&v, &text), text);
                    if turns.len() > before {
                        turns[before].id = uuid_of(&v);
                    }
                } else if let (Some(serde_json::Value::Array(blocks)), Some(turn)) = (content, turns.last_mut()) {
                    answered(turn, &v, blocks, at);
                }
            }
            // 2.1.318 writes a local command's name and its output as
            // `system` entries (`local_command`), where 2.1.316 wrote `user`
            // ones: the same wrappers, read the same way.
            Some("system") if v.get("subtype").and_then(|s| s.as_str()) == Some("local_command") => {
                let Some(text) = v.get("content").and_then(|c| c.as_str()) else { continue };
                let before = turns.len();
                absorb(&mut turns, at, local_command(text.trim()), text.to_string());
                if turns.len() > before {
                    turns[before].id = uuid_of(&v);
                }
            }
            Some("assistant") => {
                let Some(serde_json::Value::Array(blocks)) = content else { continue };
                let turn = current(&mut turns, at, &uuid_of(&v));
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => turn.say(at, b.get("text").and_then(|t| t.as_str()).unwrap_or("")),
                        Some("tool_use") => {
                            if let Some(name) = b.get("name").and_then(|n| n.as_str()) {
                                turn.call(at, name, b.get("id").and_then(|i| i.as_str()).unwrap_or(""), b.get("input"));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let mut inserted = vec![0; turns.len()];
    for q in queued.into_iter().filter(|q| {
        absorbed.get(&q.source).is_some_and(|ids| ids.contains(&q.delivery))
    }) {
        turns[q.turn].steps.insert(q.step + inserted[q.turn], Step::User { at: q.at, text: q.text });
        inserted[q.turn] += 1;
    }
    unique_ids(&mut turns);
    turns
}

/// An id names one turn: a later turn that carries an earlier one's (a
/// transcript that repeats a `uuid`, a rollout with no `task_started` before a
/// prompt) loses it, rather than answering a lookup with the wrong turn.
fn unique_ids(turns: &mut [Turn]) {
    let mut seen = HashSet::new();
    for t in turns {
        if !t.id.is_empty() && !seen.insert(t.id.clone()) {
            t.id.clear();
        }
    }
}

/// A Claude entry's own `uuid`, the id of the turn it starts.
fn uuid_of(v: &serde_json::Value) -> String {
    v.get("uuid").and_then(|u| u.as_str()).filter(|u| u.len() <= 64 && u.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).unwrap_or("").to_string()
}

enum ClaudeEntry { Person, Notice(Notice), Skip }

/// One of Claude's user (or 2.1.318 `local_command`) entries into the turns:
/// a person's line starts a turn, a command's output joins the command it
/// follows, a command's wrapper takes over the literal line `/compact`
/// writes first, and a shell's output joins its `!` line.
fn absorb(turns: &mut Vec<Turn>, at: Option<i64>, entry: ClaudeEntry, text: String) {
    match entry {
        ClaudeEntry::Person => turns.push(Turn::new(at, Some(pasted_body(&text).map_or(text.clone(), str::to_string)))),
        ClaudeEntry::Notice(Notice::Command { name, args, output }) if name.is_empty() => {
            if let Some(t) = turns.iter_mut().rev().find(|t| matches!(t.notice, Some(Notice::Command { .. }))) {
                if let Some(Notice::Command { output: o, .. }) = t.notice.as_mut() {
                    *o = output;
                }
                t.user = t.notice.as_ref().map(Notice::fallback);
            } else if output.is_some() {
                turns.push(Turn::notice(at, Notice::Command { name, args, output }));
            }
        }
        ClaudeEntry::Notice(Notice::Command { name, args, output }) => {
            // /compact writes the person's literal command before
            // the confirming wrapper, sometimes with a summary
            // between them. The wrapper proves what that line was.
            let typed = format!("/{name}{}{}", if args.is_empty() { "" } else { " " }, args);
            if let Some(t) = turns.iter_mut().rev().find(|t| t.notice.is_none() && t.user.as_deref() == Some(&typed)) {
                let notice = Notice::Command { name, args, output };
                t.user = Some(notice.fallback());
                t.notice = Some(notice);
            } else {
                turns.push(Turn::notice(at, Notice::Command { name, args, output }));
            }
        }
        ClaudeEntry::Notice(Notice::Shell { command, output, stderr }) if command.is_empty() => {
            if let Some(t) = turns.iter_mut().rev().find(|t| matches!(t.notice, Some(Notice::Shell { .. }))) {
                if let Some(Notice::Shell { output: o, stderr: e, .. }) = t.notice.as_mut() {
                    *o = output;
                    *e = stderr;
                }
                t.user = t.notice.as_ref().map(Notice::fallback);
            } else {
                turns.push(Turn::notice(at, Notice::Shell { command, output, stderr }));
            }
        }
        ClaudeEntry::Notice(notice) => turns.push(Turn::notice(at, notice)),
        ClaudeEntry::Skip => {}
    }
}

/// A 2.1.318 `local_command` entry: a command's wrapper or its output, and
/// nothing else (an entry of any other shape is skipped, never a person's).
fn local_command(t: &str) -> ClaudeEntry {
    if let Some((name, args)) = command_wrapper(t) {
        return ClaudeEntry::Notice(Notice::Command { name: one_line(name.trim_start_matches('/')).unwrap_or_default(), args: one_line(args).unwrap_or_default(), output: None });
    }
    if let Some(output) = whole_tag(t, "local-command-stdout") {
        return ClaudeEntry::Notice(Notice::Command { name: String::new(), args: String::new(), output: command_output(output) });
    }
    ClaudeEntry::Skip
}

/// A command's printed output as a client shows it, in a block: its lines
/// kept, its terminal styling (ANSI CSI and OSC sequences) and every other
/// control or invisible character gone, clipped. `None` when nothing is left.
fn command_output(text: &str) -> Option<String> {
    let mut clean = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => match it.next() {
                // CSI: parameters and intermediates, then one final byte.
                Some('[') => {
                    for d in it.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&d) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ST (ESC \\).
                Some(']') => {
                    while let Some(d) = it.next() {
                        if d == '\u{7}' || (d == '\u{1b}' && it.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' => clean.push('\n'),
            '\t' => clean.push(' '),
            c if c.is_control() || crate::harness::invisible(c) => {}
            c => clean.push(c),
        }
    }
    let lines: Vec<&str> = clean.lines().map(str::trim_end).collect();
    let text = lines.join("\n");
    let text = text.trim_matches('\n');
    (!text.trim().is_empty()).then(|| clip(text.trim_start_matches(' ')).0)
}

/// Whole-wrapper parser for Claude's injected user entries. The human origin
/// wins even when a person types XML verbatim. The peer origin is checked
/// before `isMeta`, since 2.1.316 marks peer messages as meta.
fn claude_entry(v: &serde_json::Value, text: &str) -> ClaudeEntry {
    let origin = v.get("origin");
    let kind = origin.and_then(|o| o.get("kind")).and_then(|k| k.as_str());
    if kind == Some("human") {
        // A prompt command (2.1.318, `/hello world`): Claude's whole wrapper,
        // `<command-message>` first, on the person's own entry.
        if let Some((name, args)) = prompt_command(text.trim()) {
            return ClaudeEntry::Notice(Notice::Command { name: one_line(name.trim_start_matches('/')).unwrap_or_default(), args: one_line(args).unwrap_or_default(), output: None });
        }
        return ClaudeEntry::Person;
    }
    if kind == Some("peer") {
        let from_name = origin.and_then(|o| o.get("name")).and_then(|s| s.as_str()).and_then(one_line).unwrap_or_else(|| "another session".into());
        let body = origin.and_then(|o| o.get("body")).and_then(|s| s.as_str()).unwrap_or("");
        return ClaudeEntry::Notice(Notice::PeerMessage { from_name, text: safe_text(body) });
    }
    if v.get("isCompactSummary").and_then(|s| s.as_bool()) == Some(true) {
        return ClaudeEntry::Notice(Notice::Summary { text: safe_text(&without_reminders(text)) });
    }
    if kind == Some("task-notification") || whole_tag(text, "task-notification").is_some() {
        if let Some(body) = whole_tag(text, "task-notification") {
            return ClaudeEntry::Notice(Notice::Task {
                status: tag(body, "status").and_then(one_line).unwrap_or_else(|| "updated".into()),
                summary: tag(body, "summary").and_then(one_line).unwrap_or_else(|| "Background task".into()),
            });
        }
        return ClaudeEntry::Skip;
    }
    if v.get("isMeta").and_then(|s| s.as_bool()) == Some(true) { return ClaudeEntry::Skip; }
    let t = text.trim();
    if t == "[Request interrupted by user]" || t == "[Request interrupted by user for tool use]" {
        return ClaudeEntry::Notice(Notice::Interrupt);
    }
    if let Some(command) = whole_tag(t, "bash-input") {
        return ClaudeEntry::Notice(Notice::Shell { command: one_line(command).unwrap_or_default(), output: None, stderr: None });
    }
    if let Some(output) = tag(t, "bash-stdout") {
        if let Some(rest) = t.strip_prefix(&format!("<bash-stdout>{output}</bash-stdout>")) {
            if let Some(stderr) = whole_tag(rest, "bash-stderr") {
                return ClaudeEntry::Notice(Notice::Shell { command: String::new(), output: Some(safe_text(output)), stderr: Some(safe_text(stderr)) });
            }
        }
    }
    if let Some((name, args)) = command_wrapper(t) {
        return ClaudeEntry::Notice(Notice::Command { name: one_line(name.trim_start_matches('/')).unwrap_or_default(), args: one_line(args).unwrap_or_default(), output: None });
    }
    if let Some(name) = whole_tag(t, "command-name") {
        return ClaudeEntry::Notice(Notice::Command { name: one_line(name.trim_start_matches('/')).unwrap_or_default(), args: String::new(), output: None });
    }
    if let Some(output) = whole_tag(t, "local-command-stdout") {
        return ClaudeEntry::Notice(Notice::Command { name: String::new(), args: String::new(), output: command_output(output) });
    }
    if whole_tag(t, "system-reminder").is_some() || whole_tag(t, "local-command-caveat").is_some() { return ClaudeEntry::Skip; }
    if t.is_empty() { ClaudeEntry::Skip } else { ClaudeEntry::Person }
}

/// What a person pasted, when the WHOLE message is one `<pasted_content
/// id="x">` wrapper with matching ids: how Claude records a paste it collapsed
/// (2.1.316, 2.1.318), which shows as tags in a chat and never matches the line
/// that was sent. For display and matching only. Words around the wrapper are
/// the person's own and leave it as it is, as does a body that holds its close.
fn pasted_body(text: &str) -> Option<&str> {
    let rest = text.trim().strip_prefix("<pasted_content id=\"")?;
    let (id, rest) = rest.split_once("\">\n")?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let close = format!("\n</pasted_content id=\"{id}\">");
    let body = rest.strip_suffix(close.as_str())?;
    (!body.contains("</pasted_content")).then_some(body)
}

fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(&text[start..end])
}

fn whole_tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let t = text.trim();
    let body = tag(t, name)?;
    (t == format!("<{name}>{body}</{name}>")).then_some(body)
}

/// A prompt command's wrapper as 2.1.318 writes it on the person's entry:
/// `<command-message>x</command-message>`, `<command-name>/x</command-name>`
/// and an optional `<command-args>…</command-args>`, each on its own line,
/// and nothing else.
fn prompt_command(text: &str) -> Option<(&str, &str)> {
    let (message, rest) = text.strip_prefix("<command-message>")?.split_once("</command-message>")?;
    let (name, rest) = rest.trim_start().strip_prefix("<command-name>")?.split_once("</command-name>")?;
    let rest = rest.trim();
    let args = if rest.is_empty() {
        ""
    } else {
        let (args, tail) = rest.strip_prefix("<command-args>")?.split_once("</command-args>")?;
        if !tail.trim().is_empty() {
            return None;
        }
        args
    };
    (message.trim() == name.trim_start_matches('/') && name.starts_with('/')).then_some((name, args))
}

fn command_wrapper(text: &str) -> Option<(&str, &str)> {
    let (name, rest) = text.strip_prefix("<command-name>")?.split_once("</command-name>")?;
    let (message, rest) = rest.trim_start().strip_prefix("<command-message>")?.split_once("</command-message>")?;
    let (args, rest) = rest.trim_start().strip_prefix("<command-args>")?.split_once("</command-args>")?;
    (rest.trim().is_empty() && message.trim() == name.trim_start_matches('/')).then_some((name, args))
}

fn without_reminders(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<system-reminder>") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "<system-reminder>".len()..];
        let Some(end) = after.find("</system-reminder>") else { return out };
        rest = &after[end + "</system-reminder>".len()..];
    }
    out.push_str(rest);
    out
}

fn safe_text(text: &str) -> String {
    let clean: String = text.chars().filter_map(|c| match c {
        '\n' | '\r' | '\t' => Some(' '),
        c if c.is_control() || crate::harness::invisible(c) => None,
        c => Some(c),
    }).collect();
    clip(clean.trim()).0
}

/// A user entry carrying tool results: each answers the step its
/// `tool_use_id` names in `turn` — a question's answers, a plan's verdict, or
/// a tool call a person decided at its prompt. A call no person was asked
/// about (`permissionDecision.source` is not `user_*`, and nothing refused
/// it) stays a `tool`. A text block beside a result a person allowed is
/// what they said with their Yes (Claude's `Tab to amend`).
fn answered(turn: &mut Turn, v: &serde_json::Value, blocks: &[serde_json::Value], at: Option<i64>) {
    let results: Vec<&serde_json::Value> = blocks.iter().filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result")).collect();
    let said: Vec<&str> = blocks.iter().filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")).filter_map(|b| b.get("text").and_then(|t| t.as_str())).map(str::trim).filter(|t| !t.is_empty()).collect();
    let said = (!said.is_empty() && results.len() == 1).then(|| said.join("\n"));
    let decision = v.get("permissionDecision");
    let source = decision.and_then(|d| d.get("source")).and_then(|s| s.as_str()).unwrap_or("");
    let refused = v.get("toolDenialKind").and_then(|k| k.as_str()) == Some("user-rejected")
        || decision.and_then(|d| d.get("decision")).and_then(|d| d.as_str()) == Some("reject");
    let feedback = v.get("userFeedback").and_then(|f| f.as_str()).map(str::trim).filter(|f| !f.is_empty()).map(str::to_string);
    let result = v.get("toolUseResult");
    for r in results {
        let Some(id) = r.get("tool_use_id").and_then(|i| i.as_str()) else { continue };
        let Some(step) = turn.steps.iter_mut().find(|s| s.id() == Some(id)) else { continue };
        match step {
            Step::Question { answered_at, questions, answer, .. } => {
                *answered_at = at;
                *answer = if refused {
                    let chat = feedback.as_deref().is_some_and(|f| f.starts_with("The user wants to clarify"));
                    Some(Answer { how: if chat { "chat" } else { "declined" }, answers: Vec::new() })
                } else {
                    // A preview question's note is beside its answer, in
                    // `annotations.<question>.notes` (2.1.316).
                    let notes = result.and_then(|r| r.get("annotations")).and_then(|a| a.as_object());
                    let note = |q: &Question| {
                        let n = notes?.get(&q.question)?.get("notes")?.as_str()?.trim();
                        (!n.is_empty()).then(|| n.to_string())
                    };
                    result.and_then(|r| r.get("answers")).and_then(|a| a.as_object()).map(|given| Answer {
                        how: "picked",
                        answers: questions
                            .iter()
                            .filter_map(|q| {
                                let a = given.get(&q.question)?.as_str()?;
                                let note = note(q);
                                // Enter in the notes field before the highlight
                                // ever moved sends no option at all.
                                let mut qa = match a {
                                    NOTES_ONLY if note.is_some() => QuestionAnswer { question: q.question.clone(), picked: Vec::new(), typed: None, note: None },
                                    _ => picked(q, a),
                                };
                                qa.note = note;
                                Some(qa)
                            })
                            .collect(),
                    })
                };
            }
            Step::Plan { answered_at, approved, note, plan_file, .. } => {
                *answered_at = at;
                *approved = Some(!refused);
                if refused {
                    *note = feedback.clone();
                } else if let Some(f) = result.and_then(|r| r.get("filePath")).and_then(|f| f.as_str()) {
                    *plan_file = Some(f.to_string());
                }
            }
            Step::Tool { at: called, name, summary, .. } if refused || source.starts_with("user") => {
                let allowed = !refused;
                *step = Step::Permission {
                    at: *called,
                    answered_at: at,
                    tool: std::mem::take(name),
                    summary: summary.take(),
                    allowed,
                    scope: allowed.then(|| decision.and_then(|d| d.get("grantScope")).and_then(|g| g.as_str()).map(str::to_string)).flatten(),
                    note: if allowed { said.clone() } else { feedback.clone() },
                };
            }
            // Claude 2.1.316 writes `is_error: true` on a command that
            // exited non-zero, a Read of a missing file and an interrupt;
            // a success has `false` or no key (`claude-2.1.316-tools.jsonl`).
            Step::Tool { failed, .. } => *failed = r.get("is_error").and_then(|e| e.as_bool()) == Some(true),
            _ => {}
        }
    }
}

/// What Claude writes as the answer to a preview question sent from its
/// notes field before any option was moved to: no option, only the note.
const NOTES_ONLY: &str = "(notes only)";

/// A question's answer as Claude wrote it (`"Banana"`, `"Cheese, Mushrooms"`,
/// or the text typed instead) as options picked and text typed. A
/// multi-select's labels are joined with ", ", and a label may hold one
/// itself, so the longest run of parts that is a label is taken first.
fn picked(q: &Question, given: &str) -> QuestionAnswer {
    let is_label = |s: &str| q.options.iter().any(|o| o.label == s);
    let (mut picked, mut typed) = (Vec::new(), Vec::new());
    if !q.multi || is_label(given) {
        if is_label(given) {
            picked.push(given.to_string());
        } else {
            typed.push(given);
        }
    } else {
        let parts: Vec<&str> = given.split(", ").collect();
        let mut i = 0;
        while i < parts.len() {
            match (i..parts.len()).rev().find(|&j| is_label(&parts[i..=j].join(", "))) {
                Some(j) => {
                    picked.push(parts[i..=j].join(", "));
                    i = j + 1;
                }
                None => {
                    typed.push(parts[i]);
                    i += 1;
                }
            }
        }
    }
    let typed = (!typed.is_empty()).then(|| typed.join(", "));
    QuestionAnswer { question: q.question.clone(), picked, typed, note: None }
}

/// `AskUserQuestion`'s `input.questions`.
fn questions_of(input: Option<&serde_json::Value>) -> Vec<Question> {
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    input
        .and_then(|i| i.get("questions"))
        .and_then(|q| q.as_array())
        .map(|qs| {
            qs.iter()
                .map(|q| Question {
                    key: s(q, "id"),
                    header: s(q, "header"),
                    question: s(q, "question"),
                    multi: q.get("multiSelect").and_then(|m| m.as_bool()).unwrap_or(false),
                    options: q
                        .get("options")
                        .and_then(|o| o.as_array())
                        .map(|os| os.iter().map(|o| QuestionOption { label: s(o, "label"), description: s(o, "description") }).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What a Claude tool call does, in a line ([`one_line`]): a command's first
/// line, the file it writes or reads, the address it fetches, a search's
/// pattern (and where, for Grep), a sub-agent's description, the skill
/// it loads, a todo list's length. Nothing for any other tool: an MCP tool's
/// (`mcp__server__tool`) arguments can be anything, and its name already
/// says what it is.
fn summary_of(name: &str, input: Option<&serde_json::Value>) -> Option<String> {
    let input = input?;
    let s = |k: &str| input.get(k).and_then(|v| v.as_str());
    match name {
        "Bash" => one_line(s("command")?),
        "Edit" | "MultiEdit" | "Write" | "Read" => one_line(s("file_path")?),
        "NotebookEdit" => one_line(s("notebook_path")?),
        "WebFetch" => one_line(s("url")?),
        "Grep" => {
            let pattern = one_line(s("pattern")?)?;
            match s("path").and_then(one_line) {
                Some(path) => one_line(&format!("{pattern} in {path}")),
                None => Some(pattern),
            }
        }
        "Glob" => one_line(s("pattern")?),
        "WebSearch" => one_line(s("query")?),
        "Task" | "Agent" => one_line(s("description")?),
        "Skill" => one_line(s("skill")?),
        "TodoWrite" => {
            let n = input.get("todos")?.as_array()?.len();
            Some(format!("{n} {}", if n == 1 { "item" } else { "items" }))
        }
        _ => None,
    }
}

/// Tools whose input holds a file's contents (`content`, `old_string`,
/// `new_string`, `new_source`, `edits`), each with the one field of it that is
/// kept: the path. [`ran_of`] keeps nothing else, as the contents stay in the
/// terminal. Without this list the generic input below would send them by
/// default. It goes by Claude's own tool names and not by an input's shape: an
/// MCP tool handed a file's contents shows them as it shows any input.
const PATH_ONLY: [(&str, &str); 4] = [("Edit", "file_path"), ("MultiEdit", "file_path"), ("Write", "file_path"), ("NotebookEdit", "notebook_path")];

/// What a Claude tool call was given: the text [`summary_of`] shows a line of
/// for a tool it knows (a sub-agent's being the prompt it was given, its
/// command), only the path of a [`PATH_ONLY`] tool, and for any other tool, an
/// MCP tool's included, its input.
fn ran_of(name: &str, input: Option<&serde_json::Value>) -> Ran {
    let Some(input) = input else { return Ran::Nothing };
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::to_string);
    if let Some((_, key)) = PATH_ONLY.iter().find(|(tool, _)| *tool == name) {
        return s(key).map_or(Ran::Nothing, |path| Ran::Parts(vec![Some(path)]));
    }
    let text = match name {
        "Bash" => s("command"),
        "Read" => s("file_path"),
        "WebFetch" => s("url"),
        "Grep" => s("pattern").map(|pattern| match s("path") {
            Some(path) => format!("{pattern} in {path}"),
            None => pattern,
        }),
        "Glob" => s("pattern"),
        "WebSearch" => s("query"),
        "Task" | "Agent" => s("prompt"),
        _ => return ran_json(input),
    };
    text.map_or(Ran::Nothing, |t| Ran::Parts(vec![Some(t)]))
}

/// An input as [`Ran::Json`], or nothing when it says nothing (`{}`, `null`).
fn ran_json(input: &serde_json::Value) -> Ran {
    use serde_json::Value;
    let empty = match input {
        Value::Null => true,
        Value::Object(o) => o.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::String(s) => s.is_empty(),
        _ => false,
    };
    if empty { Ran::Nothing } else { Ran::Json(input.clone()) }
}

/// `v` with every string in it, keys too, [`clean_text`]ed. Keys that clean
/// alike keep both values, the later as `<key> (2)`: a hidden character must
/// not be able to take a value out of what a person reads. A key the input
/// really has is placed first, so it is never the one renamed.
fn clean_json(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::String(s) => Value::String(clean_text(s)),
        Value::Array(a) => Value::Array(a.iter().map(clean_json).collect()),
        Value::Object(o) => {
            let mut out = serde_json::Map::with_capacity(o.len());
            let (plain, hidden): (Vec<_>, Vec<_>) = o.iter().partition(|(k, _)| clean_text(k) == **k);
            for (k, v) in plain.into_iter().chain(hidden) {
                let key = clean_text(k);
                let mut free = key.clone();
                let mut n = 2;
                while out.contains_key(&free) {
                    free = format!("{key} ({n})");
                    n += 1;
                }
                out.insert(free, clean_json(v));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Line breaks and tabs stay, a carriage return is a line break (alone, it
/// would draw over the line it ends), and every other control character and
/// invisible format character goes, so what a person reads is what ran. This
/// removes characters and redacts nothing: a token in a command stays in it.
fn clean_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\r' => Some('\n'),
            '\n' | '\t' => Some(c),
            c if c.is_control() || crate::harness::invisible(c) => None,
            c => Some(c),
        })
        .collect()
}

/// A command's whole text, its parts joined and [`clean_text`]ed, keeping its
/// shape where `one_line` keeps a line.
fn parts_text(parts: &[Option<String>]) -> Option<String> {
    if !parts.iter().any(Option::is_some) {
        return None;
    }
    let joined = parts.iter().map(|p| p.as_deref().unwrap_or(BUILT_COMMAND)).collect::<Vec<_>>().join("\n\n");
    let clean = clean_text(&joined);
    let text = clean.trim_matches('\n').trim_end();
    (!text.trim().is_empty()).then(|| text.to_string())
}

/// What stands for a command a script builds and no event has named.
const BUILT_COMMAND: &str = "(a command this script builds)";

/// `agent_tool_call` would say more about this call than `summary` does.
fn says_more(ran: &Ran, summary: Option<&str>) -> bool {
    match ran {
        Ran::Nothing => false,
        Ran::Json(input) => !repeats_line(input, summary),
        Ran::Parts(parts) => parts_text(parts).is_some_and(|text| {
            text.lines().filter(|l| !l.trim().is_empty()).count() > 1 || text.trim().chars().count() > SUMMARY_MAX || one_line(&text).as_deref() != summary
        }),
    }
}

/// An input with no values at all, or whose one value is the line already
/// shown (`Skill`'s `{"skill": "x"}`), which opening would only say again.
fn repeats_line(input: &serde_json::Value, summary: Option<&str>) -> bool {
    use serde_json::Value;
    fn leaves<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| leaves(x, out)),
            Value::Object(o) => o.values().for_each(|x| leaves(x, out)),
            other => out.push(other),
        }
    }
    let mut all = Vec::new();
    leaves(input, &mut all);
    match (all.as_slice(), summary) {
        ([], _) => true,
        ([Value::String(s)], Some(line)) => one_line(s).as_deref() == Some(line) && s.trim().chars().count() <= SUMMARY_MAX && s.trim().lines().count() <= 1,
        _ => false,
    }
}

/// One call's input, `agent_tool_call`'s answer. `input` is the whole text
/// for a command, a path, a query or a sub-agent's prompt, and the structured
/// value for any other tool. Past [`CALL_INPUT_MAX`] it is a string either
/// way, cut on a character: a value cut there would not parse, so it comes
/// back as its pretty-printed text, and `truncated` says so.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CallDetail {
    pub tool: String,
    pub input: serde_json::Value,
    pub truncated: bool,
    /// The input's whole length in bytes, as text, cut or not.
    pub bytes: usize,
}

/// The input of the Claude call `id`, from the user and assistant entries
/// [`claude`] reads, or None when it has none or has left `lines`.
pub fn claude_tool_call(lines: &[String], id: &str) -> Option<CallDetail> {
    detail_in(claude_turns(lines), id)
}

/// The picture a Claude Read call was SHOWN (`feat/chat-images`): the
/// `tool_result` image block of the Read whose `tool_use` id is `id`, as Claude
/// 2.1.319 writes it (`claude-2.1.319-read-image.jsonl`). `b64` is what the
/// model saw, recompressed, not the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallImage {
    pub mime: String,
    /// Empty when `truncated`: a block past [`CALL_IMAGE_MAX_B64`] is never carried.
    pub b64: String,
    /// Decoded bytes the block holds.
    pub size: u64,
    pub truncated: bool,
}

/// The most base64 a [`CallImage`] carries: the 4 MB `read_file_base64` cap, as
/// base64.
pub const CALL_IMAGE_MAX_B64: usize = 5_400_000;

/// The image of the Claude Read call `id` in `lines`, or None when `id` names no
/// Read, its result holds no image block, the type is not one the viewer draws
/// (png, jpeg, gif, webp) or it has left `lines`. Reads `user` and `assistant`
/// entries by id only; nothing else of a result is returned.
pub fn claude_call_image(lines: &[String], id: &str) -> Option<CallImage> {
    use serde_json::Value;
    if id.is_empty() {
        return None;
    }
    let mut is_read = false;
    let mut found = None;
    for line in lines.iter().filter(|l| l.contains(id)) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(items) = v.pointer("/message/content").and_then(|c| c.as_array()) else { continue };
        for it in items {
            match it.get("type").and_then(|t| t.as_str()) {
                Some("tool_use") if it["id"] == id && v["type"] == "assistant" => is_read = it["name"] == "Read",
                Some("tool_result") if it["tool_use_id"] == id && v["type"] == "user" => {
                    let img = it.get("content").and_then(|c| c.as_array()).and_then(|a| a.iter().find(|b| b["type"] == "image"));
                    if let Some(src) = img.and_then(|b| b.get("source")).filter(|s| s["type"] == "base64") {
                        found = Some((src["media_type"].as_str().unwrap_or("").to_string(), src["data"].as_str().unwrap_or("").to_string()));
                    }
                }
                _ => {}
            }
        }
    }
    let (mime, data) = found?;
    if !is_read || !["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&mime.as_str()) || data.is_empty() {
        return None;
    }
    let size = (data.len() / 4 * 3) as u64;
    let truncated = data.len() > CALL_IMAGE_MAX_B64;
    Some(CallImage { mime, b64: if truncated { String::new() } else { data }, size, truncated })
}

/// The input of the Codex call `id`, as [`claude_tool_call`].
pub fn codex_tool_call(lines: &[String], id: &str) -> Option<CallDetail> {
    detail_in(codex_turns(lines), id)
}

fn detail_in(turns: Vec<Turn>, id: &str) -> Option<CallDetail> {
    if id.is_empty() {
        return None;
    }
    let (tool, ran) = turns.iter().flat_map(|t| &t.steps).find_map(|s| match s {
        Step::Tool { id: i, name, ran, .. } if i == id => Some((name, ran)),
        _ => None,
    })?;
    use serde_json::Value;
    let (whole, value) = match ran {
        Ran::Nothing => return None,
        Ran::Parts(parts) => (parts_text(parts)?, None),
        Ran::Json(input) => {
            let clean = clean_json(input);
            (serde_json::to_string_pretty(&clean).ok()?, Some(clean))
        }
    };
    let bytes = whole.len();
    let truncated = bytes > CALL_INPUT_MAX;
    let input = match value {
        Some(v) if !truncated => v,
        _ => {
            let mut end = bytes.min(CALL_INPUT_MAX);
            while !whole.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(whole[..end].to_string())
        }
    };
    Some(CallDetail { tool: tool.clone(), input, truncated, bytes })
}

/// `text`'s first line that holds anything, as a summary: every control
/// character removed (a tab becomes a space) and every invisible format
/// character too — `harness::invisible`, the set a typed line refuses (bidi
/// controls, zero-width characters: Trojan Source's class) — then trimmed
/// and clipped to [`SUMMARY_MAX`]. A summary is transcript text, which a
/// repository or a prompt can steer, so what a person reads is what is there.
fn one_line(text: &str) -> Option<String> {
    text.lines()
        .map(|l| {
            l.chars()
                .filter_map(|c| match c {
                    '\t' => Some(' '),
                    c if c.is_control() || crate::harness::invisible(c) => None,
                    c => Some(c),
                })
                .collect::<String>()
        })
        .find(|l| !l.trim().is_empty())
        .map(|l| clip_to(l.trim(), SUMMARY_MAX).0)
}

/// What a Codex tool call does, in a line: a command, or the files a patch
/// touches. Codex 0.160.1 runs every tool through "code mode": ONE custom
/// tool, `exec`, whose input is a JavaScript script calling
/// `tools.exec_command({cmd: "…"})` or `tools.apply_patch("…")`
/// (`codex-0.160.1-tools.jsonl`; the model refuses to work with code mode
/// off). Older shapes: `exec_command` / `shell_command` / `shell` function
/// calls, whose arguments carry the command (`shell`'s as an argv), and an
/// `apply_patch` call carrying the patch. Nothing for any other tool.
fn codex_summary(name: &str, p: &serde_json::Value) -> Option<String> {
    let args = || p.get("arguments").and_then(|a| a.as_str()).and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok());
    match name {
        "exec" => code_mode_summary(p.get("input")?.as_str()?),
        "apply_patch" => match p.get("input").and_then(|i| i.as_str()) {
            Some(patch) => patch_summary(patch),
            None => patch_summary(args()?.get("input")?.as_str()?),
        },
        "exec_command" | "shell_command" | "shell" => {
            let a = args()?;
            match a.get("cmd").or_else(|| a.get("command"))? {
                serde_json::Value::String(c) => one_line(c),
                serde_json::Value::Array(argv) => argv_summary(argv),
                _ => None,
            }
        }
        _ => None,
    }
}

/// What a Codex tool call was given, in the shapes [`codex_summary`] reads (a
/// patch is the files it touches and never its text), or any other tool's
/// input: a function call's arguments, a custom tool's own text.
fn codex_ran(name: &str, p: &serde_json::Value) -> Ran {
    let args = || p.get("arguments").and_then(|a| a.as_str()).and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok());
    let text = match name {
        "exec" => return p.get("input").and_then(|i| i.as_str()).map_or(Ran::Nothing, |js| Ran::Parts(code_mode_ran(js))),
        "apply_patch" => match p.get("input").and_then(|i| i.as_str()) {
            Some(patch) => patch_files(patch),
            None => args().and_then(|a| a.get("input").and_then(|i| i.as_str()).and_then(patch_files)),
        },
        "exec_command" | "shell_command" | "shell" => args().and_then(|a| match a.get("cmd").or_else(|| a.get("command")) {
            Some(serde_json::Value::String(c)) => Some(c.clone()),
            Some(serde_json::Value::Array(argv)) => argv_text(argv),
            _ => None,
        }),
        _ => match (p.get("input").and_then(|i| i.as_str()), args()) {
            (Some(input), _) => Some(input.to_string()),
            (None, Some(a)) => return ran_json(&a),
            (None, None) => p.get("arguments").and_then(|a| a.as_str()).map(str::to_string),
        },
    };
    text.map_or(Ran::Nothing, |t| Ran::Parts(vec![Some(t)]))
}

/// A command's argv as a person typed it: `["/bin/zsh", "-lc", "<cmd>"]`
/// (what Codex 0.160.1 runs) is `<cmd>`; anything else, its words.
fn argv_text(argv: &[serde_json::Value]) -> Option<String> {
    let words: Vec<&str> = argv.iter().filter_map(|w| w.as_str()).collect();
    if words.is_empty() {
        return None;
    }
    Some(match words.as_slice() {
        [sh, "-lc" | "-c", cmd] if matches!(sh.rsplit('/').next(), Some("bash" | "zsh" | "sh")) => cmd.to_string(),
        _ => words.join(" "),
    })
}

/// [`argv_text`]'s line.
fn argv_summary(argv: &[serde_json::Value]) -> Option<String> {
    one_line(&argv_text(argv)?)
}

/// A code-mode script, by the first tool it calls: `exec_command`'s `cmd`,
/// `apply_patch`'s patch, or any other tool's name. Only a string LITERAL is
/// read; a command the script builds is left to its `CommandExecution`
/// event ([`codex`]). A script calling more than one tool says how many
/// more, so a chip never hides the rest.
fn code_mode_summary(js: &str) -> Option<String> {
    let calls = code_calls(js);
    let &(name, args) = calls.first()?;
    let what = match name {
        "exec_command" => key_literal(&js[args..], "cmd").and_then(|c| one_line(&c)),
        "apply_patch" => js_literal(js[args..].trim_start()).and_then(|(patch, _)| patch_summary(&patch)),
        other => one_line(other),
    }?;
    Some(match calls.len() - 1 {
        0 => what,
        more => format!("{what} (+{more} more)"),
    })
}

/// Each tool a code-mode script calls, in order: `exec_command`'s `cmd`
/// (None when the script builds it), the files of an `apply_patch`, or any
/// other tool's call as the script wrote it, its arguments' source included.
fn code_mode_ran(js: &str) -> Vec<Option<String>> {
    code_calls(js)
        .into_iter()
        .map(|(name, args)| match name {
            "exec_command" => key_literal(&js[args..], "cmd"),
            "apply_patch" => js_literal(js[args..].trim_start()).and_then(|(patch, _)| patch_files(&patch)),
            other => Some(format!("{other}({})", call_args(&js[args..]).trim())),
        })
        .collect()
}

/// The source of a call's arguments: `s` up to the `)` that closes it, string
/// literals skipped whole. All of `s` when it never closes.
fn call_args(s: &str) -> &str {
    let b = s.as_bytes();
    let (mut i, mut depth) = (0, 0usize);
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' | b'`' => {
                i += js_literal(&s[i..]).map_or(1, |(_, n)| n);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' if depth == 0 => return &s[..i],
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    s
}

/// Each `tools.<name>(` a code-mode script calls: the name, and where its
/// arguments start. String literals are skipped whole, so a patch whose
/// text calls `tools.…(` is one call, not two.
fn code_calls(js: &str) -> Vec<(&str, usize)> {
    let b = js.as_bytes();
    let mut calls = Vec::new();
    let mut i = 0;
    // Only ASCII bytes are matched, so every slice starts on a char boundary.
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' | b'`' => i += js_literal(&js[i..]).map_or(1, |(_, n)| n),
            b't' if js[i..].starts_with("tools.") && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || matches!(b[i - 1], b'_' | b'.' | b'$'))) => {
                let start = i + "tools.".len();
                let after = start + js[start..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(js.len() - start);
                let rest = js[after..].trim_start();
                if after > start && rest.starts_with('(') {
                    calls.push((&js[start..after], js.len() - rest.len() + 1));
                }
                i = after;
            }
            _ => i += 1,
        }
    }
    calls
}

/// The string literal given for `key` in the object literal at the start of
/// `s` (`{cmd: "ls"}`, `{"cmd": 'ls'}`), or None.
fn key_literal(s: &str, key: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = s[from..].find(key) {
        let at = from + i;
        from = at + key.len();
        let before = s[..at].chars().next_back();
        if !matches!(before, Some('{' | ',' | '"' | '\'' | ' ' | '\n' | '\t')) {
            continue;
        }
        let rest = s[from..].trim_start_matches(|c| c == '"' || c == '\'').trim_start();
        if let Some(value) = rest.strip_prefix(':') {
            return js_literal(value.trim_start()).map(|(v, _)| v);
        }
    }
    None
}

/// The JavaScript string literal `s` starts with, decoded, and its length in
/// bytes. A template literal with a `${…}` in it is not a literal.
fn js_literal(s: &str) -> Option<(String, usize)> {
    let q = s.chars().next().filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let mut out = String::new();
    let mut it = s.char_indices().skip(1).peekable();
    while let Some((i, c)) = it.next() {
        match c {
            c if c == q => return Some((out, i + 1)),
            '$' if q == '`' && it.peek().map(|&(_, n)| n) == Some('{') => return None,
            '\\' => {
                let (_, e) = it.next()?;
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'v' => out.push('\u{b}'),
                    '0' => out.push('\0'),
                    '\n' => {}
                    'x' | 'u' => {
                        let braced = e == 'u' && it.peek().map(|&(_, n)| n) == Some('{');
                        if braced {
                            it.next();
                        }
                        let mut hex = String::new();
                        while let Some(&(_, h)) = it.peek() {
                            if braced && h == '}' {
                                it.next();
                                break;
                            }
                            if !h.is_ascii_hexdigit() || (!braced && hex.len() == if e == 'x' { 2 } else { 4 }) {
                                break;
                            }
                            hex.push(h);
                            it.next();
                        }
                        out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}'));
                    }
                    other => out.push(other),
                }
            }
            c => out.push(c),
        }
    }
    None
}

/// [`patch_files`]' line.
fn patch_summary(patch: &str) -> Option<String> {
    one_line(&patch_files(patch)?)
}

/// The files a patch touches, from its `*** Update File:` / `*** Add File:`
/// / `*** Delete File:` headers: `edit a.rs, b.rs; add c.rs`.
fn patch_files(patch: &str) -> Option<String> {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for l in patch.lines() {
        let Some((verb, path)) = [("*** Update File: ", "edit"), ("*** Add File: ", "add"), ("*** Delete File: ", "delete")]
            .iter()
            .find_map(|(head, verb)| l.strip_prefix(head).map(|p| (*verb, p.trim())))
        else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(v, _)| *v == verb) {
            Some((_, paths)) => paths.push(path),
            None => groups.push((verb, vec![path])),
        }
    }
    if groups.is_empty() {
        return None;
    }
    Some(groups.iter().map(|(v, paths)| format!("{v} {}", paths.join(", "))).collect::<Vec<_>>().join("; "))
}

/// A Codex tool's output that says the call did not do its work: a
/// code-mode script that threw (`Script failed`, the head of its output;
/// a patch that does not apply is one, `codex-0.160.1-exec.jsonl`) or a call
/// stopped before it ran (`aborted by user`: a No at its approval, or Esc,
/// which Codex does not tell apart). A command's own exit status is not in
/// the output a script prints; it is in its `CommandExecution` event.
fn codex_output_failed(output: Option<&serde_json::Value>) -> bool {
    match output {
        Some(serde_json::Value::Array(blocks)) => {
            blocks.first().and_then(|b| b.get("text")).and_then(|t| t.as_str()).is_some_and(|t| t.starts_with("Script failed"))
        }
        Some(serde_json::Value::String(s)) => s.starts_with("aborted by user"),
        _ => false,
    }
}

/// The last `n` turns of a Codex rollout's `lines`: `response_item`
/// messages, a `user` one starting a turn unless it is one of the `<…>`-wrapped
/// blocks Codex injects (the environment, AGENTS.md, an image) or a
/// `# AGENTS.md instructions` block; an `assistant` one's `output_text` is
/// the reply, a `function_call` / `custom_tool_call` a tool, and its output
/// says whether it failed ([`codex_output_failed`]). `developer` and
/// `system` messages, reasoning and `session_meta` are never read.
///
/// One `event_msg` is: `item_completed` for a `CommandExecution`, which
/// Codex 0.160.1 (its TUI and `codex exec` alike) writes for each command a
/// call runs, between the call and its output, with the command's `status`
/// (`failed`) and `exit_code` — the only place a script's command says it
/// failed. It is the call's while that call is the only one awaiting its
/// output; with several in flight it is nobody's, rather than a guess. Its
/// argv fills a summary the call's own input did not give. Its output
/// (`stdout`, `formatted_output`…) is never read.
pub fn codex(lines: &[String], n: usize) -> Vec<Turn> {
    finish(codex_turns(lines), n)
}

/// Every turn in a Codex rollout's `lines`, as [`codex`] reads them.
fn codex_turns(lines: &[String]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    // The calls awaiting their output, by `call_id`, in the last turn.
    let mut open: Vec<String> = Vec::new();
    // The `turn_id` of the turn being read: `task_started` opens one (before
    // its typed prompt in every rollout measured), and a `turn_context` names
    // it for a window that starts inside a turn and so holds no `task_started`.
    let mut turn_id = String::new();
    for l in lines {
        if l.contains("\"task_started\"") || l.contains("\"turn_context\"") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
                let started = v.pointer("/payload/type").and_then(|t| t.as_str()) == Some("task_started");
                if let Some(id) = v.pointer("/payload/turn_id").and_then(|i| i.as_str()).filter(|i| i.len() <= 64 && i.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
                    if started || (turn_id.is_empty() && turns.last().is_none_or(|t| t.id != id)) {
                        turn_id = id.to_string();
                    }
                }
            }
            continue;
        }
        // A window that starts inside a turn holds neither: its first turn is
        // named by the first event that carries the id, near the line's head.
        if turns.is_empty() && turn_id.is_empty() && l.contains("\"item_completed\"") {
            if let Some(id) = l.get(..l.len().min(600)).and_then(|h| h.split_once("\"turn_id\":\"")).and_then(|(_, r)| r.split_once('"')).map(|(id, _)| id).filter(|i| i.len() <= 64 && i.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
                turn_id = id.to_string();
            }
        }
        let event = l.contains("\"item_completed\"") && l.contains("\"CommandExecution\"");
        if !event && !l.contains("\"response_item\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        let Some(p) = v.get("payload") else { continue };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("response_item") => {}
            Some("event_msg") if event && p.get("type").and_then(|t| t.as_str()) == Some("item_completed") => {
                let Some(item) = p.get("item").filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("CommandExecution")) else { continue };
                let [id] = open.as_slice() else { continue };
                if let Some(Step::Tool { failed, summary, ran, .. }) = turns.last_mut().and_then(|t| t.steps.iter_mut().find(|s| s.id() == Some(id.as_str()))) {
                    *failed |= item.get("status").and_then(|s| s.as_str()) == Some("failed") || item.get("exit_code").and_then(|c| c.as_i64()).is_some_and(|c| c != 0);
                    let argv = item.get("command").and_then(|c| c.as_array());
                    if summary.is_none() {
                        *summary = argv.and_then(|argv| argv_summary(argv));
                    }
                    // A built command's text: the next event not already a
                    // literal of the script's, for the next part it builds.
                    if let (Ran::Parts(parts), Some(cmd)) = (ran, argv.and_then(|argv| argv_text(argv))) {
                        if !parts.iter().flatten().any(|c| *c == cmd) {
                            if let Some(part) = parts.iter_mut().find(|p| p.is_none()) {
                                *part = Some(cmd);
                            }
                        }
                    }
                }
                continue;
            }
            _ => continue,
        }
        let at = v.get("timestamp").and_then(|t| t.as_str()).and_then(crate::sysclock::parse_iso8601);
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
                    let mut turn = Turn::new(at, Some(typed.join("\n")));
                    turn.id = std::mem::take(&mut turn_id);
                    turns.push(turn);
                    open.clear();
                }
            }
            (Some("message"), Some("assistant")) => {
                let turn = current(&mut turns, at, &turn_id);
                for t in texts("output_text") {
                    turn.say(at, &t);
                }
            }
            (Some("function_call" | "custom_tool_call"), _) => {
                if let Some(name) = p.get("name").and_then(|n| n.as_str()) {
                    let t = current(&mut turns, at, &turn_id);
                    t.tools.push(name.to_string());
                    let id = p.get("call_id").and_then(|c| c.as_str()).unwrap_or("").to_string();
                    if !id.is_empty() {
                        open.push(id.clone());
                    }
                    t.steps.push(if name == "request_user_input" {
                        let args = p.get("arguments").and_then(|a| a.as_str()).and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok());
                        Step::Question { at, answered_at: None, questions: questions_of(args.as_ref()), answer: None, id }
                    } else {
                        Step::Tool { at, name: name.to_string(), id, summary: codex_summary(name, p), failed: false, ran: codex_ran(name, p), detail: false }
                    });
                }
            }
            (Some("function_call_output" | "custom_tool_call_output"), _) => {
                let id = p.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                open.retain(|o| o != id);
                match turns.last_mut().and_then(|t| t.steps.iter_mut().find(|s| s.id() == Some(id))) {
                    Some(Step::Question { answered_at, questions, answer, .. }) => {
                        let out = p.get("output").and_then(|o| o.as_str()).and_then(|o| serde_json::from_str::<serde_json::Value>(o).ok());
                        *answered_at = at;
                        *answer = out.as_ref().and_then(|o| o.get("answers")).and_then(|a| a.as_object()).map(|given| Answer {
                            how: "picked",
                            answers: questions.iter().filter_map(|q| given.get(&q.key).map(|a| codex_answer(q, a))).collect(),
                        });
                    }
                    Some(Step::Tool { failed, .. }) => *failed |= codex_output_failed(p.get("output")),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    unique_ids(&mut turns);
    turns
}

/// One question's answer in a Codex `request_user_input` output: its
/// `answers` list holds the options picked, by label, any text typed
/// instead, and a note given beside them as `user_note: …`.
fn codex_answer(q: &Question, given: &serde_json::Value) -> QuestionAnswer {
    let (mut picked, mut typed, mut note) = (Vec::new(), Vec::new(), None);
    for a in given.get("answers").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|a| a.as_str()) {
        if let Some(n) = a.strip_prefix("user_note: ") {
            note = Some(n.to_string());
        } else if q.options.iter().any(|o| o.label == a) {
            picked.push(a.to_string());
        } else {
            typed.push(a);
        }
    }
    let typed = (!typed.is_empty()).then(|| typed.join(", "));
    QuestionAnswer { question: q.question.clone(), picked, typed, note }
}

/// A user `input_text` block somebody typed, not one Codex wrapped in.
fn codex_typed(text: &str) -> bool {
    let t = text.trim_start();
    !t.is_empty() && !t.starts_with('<') && !t.starts_with("# AGENTS.md instructions")
}

/// The last `n` turns of a kimi session's wire log `lines`, oldest first.
/// kimi 2.2.0 (`kimi-wire/2.2.0/transcript.jsonl`, the record shapes a real
/// session wrote, the content invented): a `turn.prompt` starts a turn; the
/// turn's story is `context.append_loop_event` events — `content.part` text
/// blocks (`think` parts are reasoning, left out like Claude's), `tool.call` /
/// `tool.result` pairs (only `isError` is read of a result, never its
/// output) — and `interaction.request` / `interaction.resolved` pairs carry
/// what a person was asked and did: an approval turns its call into a
/// `permission` step (allowed, and a session scope, from the resolution), a
/// question turns its `AskUserQuestion` call into a `question` step (the
/// answer is keyed by the question's text). The two arrive in DIFFERENT
/// orders, measured on real sessions: an approval's request lands BEFORE its
/// `tool.call` (a rejected one has no call at all — the step is built at the
/// resolution), a question's call BEFORE its request. An interaction whose
/// resolution left the read window stays un-answered, which the chat does
/// not draw: the live prompt card covers it. An Esc is `agent.turn.ended`
/// with `outcome: aborted`: the turn keeps the part that ran, with no
/// marker, as Codex's reader leaves an interrupted turn.
pub fn kimi(lines: &[String], n: usize) -> Vec<Turn> {
    finish(kimi_turns(lines), n)
}

/// Every turn in a kimi wire log's `lines`, as [`kimi`] reads them.
fn kimi_turns(lines: &[String]) -> Vec<Turn> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Ask {
        Approval,
        Question,
    }
    /// An interaction still open: matched to its `tool.call` by the call id
    /// (a permission step carries no id, so the step's place is recorded
    /// once made), and filled by the resolution.
    #[derive(Clone)]
    struct Open {
        id: String,
        tool_call: String,
        ask: Ask,
        asked_at: Option<i64>,
        tool: String,
        said: Option<String>,
        questions: Vec<Question>,
        answered_at: Option<i64>,
        allowed: Option<bool>,
        scope: Option<String>,
        answer: Option<Answer>,
        step: Option<(usize, usize)>,
    }
    let mut turns: Vec<Turn> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut apply = |open: &Open, turns: &mut Vec<Turn>| {
        let Some((ti, si)) = open.step else { return };
        let Some(step) = turns.get_mut(ti).and_then(|t| t.steps.get_mut(si)) else { return };
        match step {
            Step::Permission { answered_at, allowed, scope, .. } => {
                *answered_at = open.answered_at;
                *allowed = open.allowed.unwrap_or(false);
                *scope = open.allowed.unwrap_or(false).then(|| open.scope.clone()).flatten();
            }
            Step::Question { answered_at, answer, .. } => {
                *answered_at = open.answered_at;
                *answer = open.answer.clone();
            }
            _ => {}
        }
    };
    for l in lines {
        if !(l.contains("\"turn.prompt\"")
            || l.contains("\"context.append_loop_event\"")
            || l.contains("\"interaction.request\"")
            || l.contains("\"interaction.resolved\""))
        {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        let at = v.get("time").and_then(|t| t.as_i64()).map(|ms| ms / 1000);
        match v.get("type").and_then(|t| t.as_str()) {
            Some("turn.prompt") => {
                if v.pointer("/origin/kind").and_then(|k| k.as_str()) != Some("user") {
                    continue;
                }
                let text = v
                    .get("input")
                    .and_then(|i| i.as_array())
                    .map(|parts| parts.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""))
                    .unwrap_or_default();
                turns.push(Turn::new(at, Some(text)));
            }
            Some("context.append_loop_event") => {
                let Some(e) = v.get("event") else { continue };
                match e.get("type").and_then(|t| t.as_str()) {
                    Some("content.part") => {
                        // `think` parts are reasoning, not the reply.
                        if e.pointer("/part/type").and_then(|t| t.as_str()) != Some("text") {
                            continue;
                        }
                        let Some(text) = e.pointer("/part/text").and_then(|t| t.as_str()) else { continue };
                        current(&mut turns, at, "").say(at, text);
                    }
                    Some("tool.call") => {
                        let (Some(name), id) = (
                            e.get("name").and_then(|n| n.as_str()),
                            e.get("toolCallId").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                        ) else { continue };
                        let args = e.get("args");
                        // A call a person is asked about is its asked step,
                        // whatever order the pair arrived in: the request
                        // first (an approval's way — the resolution may have
                        // built the step already, and the call then adds
                        // nothing) or the call first (a question's way).
                        let open_i = open.iter().position(|o| o.tool_call == id);
                        if open_i.is_some_and(|i| open[i].step.is_some()) {
                            continue;
                        }
                        let asked = open_i.map(|i| open[i].clone());
                        let turn = current(&mut turns, at, "");
                        turn.tools.push(name.to_string());
                        let step = match asked {
                            Some(o) if o.ask == Ask::Question => Step::Question {
                                at,
                                answered_at: o.answered_at,
                                questions: if o.questions.is_empty() { kimi_questions(args) } else { o.questions.clone() },
                                answer: o.answer.clone(),
                                id,
                            },
                            Some(o) => Step::Permission {
                                at,
                                answered_at: o.answered_at,
                                tool: if o.tool.is_empty() { name.to_string() } else { o.tool },
                                summary: o.said.clone(),
                                allowed: o.allowed.unwrap_or(false),
                                scope: o.allowed.unwrap_or(false).then(|| o.scope.clone()).flatten(),
                                note: None,
                            },
                            None if name == "AskUserQuestion" => Step::Question { at, answered_at: None, questions: kimi_questions(args), answer: None, id },
                            None => Step::Tool {
                                at,
                                name: name.to_string(),
                                id,
                                summary: kimi_summary(name, args, e.get("display")),
                                failed: false,
                                ran: kimi_ran(name, args),
                                detail: false,
                            },
                        };
                        let si = turn.steps.len();
                        turn.steps.push(step);
                        if let Some(i) = open_i {
                            open[i].step = Some((turns.len() - 1, si));
                        }
                    }
                    Some("tool.result") => {
                        let Some(id) = e.get("toolCallId").and_then(|i| i.as_str()) else { continue };
                        if let Some(Step::Tool { failed, .. }) = current(&mut turns, at, "").steps.iter_mut().find(|s| s.id() == Some(id)) {
                            *failed = e.pointer("/result/isError").and_then(|x| x.as_bool()) == Some(true);
                        }
                    }
                    _ => {}
                }
            }
            Some("interaction.request") => {
                let (Some(id), Some(kind), tool_call) = (
                    v.get("id").and_then(|i| i.as_str()),
                    v.get("kind").and_then(|k| k.as_str()),
                    v.get("toolCallId").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                ) else { continue };
                let req = v.get("request");
                let mut o = Open {
                    id: id.to_string(),
                    tool_call: tool_call.clone(),
                    ask: if kind == "question" { Ask::Question } else { Ask::Approval },
                    asked_at: at,
                    tool: req.and_then(|r| r.get("toolName")).and_then(|t| t.as_str()).unwrap_or("").to_string(),
                    said: req.and_then(|r| r.get("action")).and_then(|a| a.as_str()).and_then(one_line),
                    questions: kimi_questions(req),
                    answered_at: None,
                    allowed: None,
                    scope: None,
                    answer: None,
                    step: None,
                };
                // Call-first ordering (a question's way): the step exists —
                // merge the request's canonical questions, or upgrade the
                // tool, and record where the resolution writes.
                let ti = turns.len().saturating_sub(1);
                if let Some(pos) = turns.last().and_then(|t| t.steps.iter().position(|s| s.id() == Some(tool_call.as_str()))) {
                    match (o.ask, &mut turns[ti].steps[pos]) {
                        (Ask::Approval, Step::Tool { at: called, name, summary, .. }) => {
                            let said = summary.take().or(o.said.take());
                            let tool = if o.tool.is_empty() { std::mem::take(name) } else { std::mem::take(&mut o.tool) };
                            turns[ti].steps[pos] = Step::Permission {
                                at: *called,
                                answered_at: None,
                                tool,
                                summary: said,
                                allowed: false,
                                scope: None,
                                note: None,
                            };
                        }
                        (Ask::Question, Step::Question { questions, .. }) => {
                            if !o.questions.is_empty() {
                                *questions = o.questions.clone();
                            }
                        }
                        _ => {}
                    }
                    o.step = Some((ti, pos));
                }
                open.push(o);
            }
            Some("interaction.resolved") => {
                let Some(id) = v.get("id").and_then(|i| i.as_str()) else { continue };
                let Some(i) = open.iter().position(|o| o.id == id) else { continue };
                let response = v.get("response");
                {
                    let o = &mut open[i];
                    o.answered_at = at;
                    if o.ask == Ask::Approval {
                        o.allowed = Some(response.and_then(|r| r.get("decision")).and_then(|d| d.as_str()) == Some("approved"));
                        o.scope = response.and_then(|r| r.get("scope")).and_then(|s| s.as_str()).map(str::to_string);
                    } else {
                        // A dismissal (Esc) resolves the question with no
                        // response at all (`kimi-wire/2.2.0/question.jsonl`).
                        o.answer = match response {
                            Some(r) => r.get("answers").and_then(|a| a.as_object()).map(|given| Answer {
                                how: "picked",
                                answers: o.questions.iter().filter_map(|q| given.get(&q.key).and_then(|a| a.as_str()).map(|a| picked(q, a))).collect(),
                            }),
                            None => Some(Answer { how: "declined", answers: Vec::new() }),
                        };
                    }
                }
                if open[i].step.is_some() {
                    apply(&open[i], &mut turns);
                } else {
                    // Request-first ordering's rejected approval: no call ever
                    // lands, so the step is born of the resolution, at the
                    // position the refusal reached.
                    let o = open[i].clone();
                    if !(o.ask == Ask::Approval && o.tool.is_empty()) {
                        let turn = current(&mut turns, o.asked_at, "");
                        if !o.tool.is_empty() {
                            turn.tools.push(o.tool.clone());
                        }
                        let si = turn.steps.len();
                        turn.steps.push(match o.ask {
                            Ask::Approval => Step::Permission {
                                at: o.asked_at,
                                answered_at: o.answered_at,
                                tool: o.tool,
                                summary: o.said,
                                allowed: o.allowed.unwrap_or(false),
                                scope: o.allowed.unwrap_or(false).then(|| o.scope.clone()).flatten(),
                                note: None,
                            },
                            Ask::Question => Step::Question { at: o.asked_at, answered_at: o.answered_at, questions: o.questions.clone(), answer: o.answer.clone(), id: o.tool_call.clone() },
                        });
                        open[i].step = Some((turns.len() - 1, si));
                    }
                }
            }
            _ => {}
        }
    }
    turns
}

/// kimi's `AskUserQuestion` arguments into questions. The resolution's
/// `answers` object is keyed by the question's own text (`key` is what the
/// answer is looked up by), and 2.2.0's questions have no multi-select.
fn kimi_questions(args: Option<&serde_json::Value>) -> Vec<Question> {
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    args.and_then(|a| a.get("questions"))
        .and_then(|q| q.as_array())
        .map(|qs| {
            qs.iter()
                .map(|q| {
                    let question = s(q, "question");
                    Question {
                        key: question.clone(),
                        header: s(q, "header"),
                        question,
                        multi: false,
                        options: q.get("options").and_then(|o| o.as_array()).map(|os| os.iter().map(|o| QuestionOption { label: s(o, "label"), description: s(o, "description") }).collect()).unwrap_or_default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What a kimi tool call does, in a line: its command, the file it touched
/// (the path as typed, where the arguments carry one — `display.path` is
/// absolute), a search's pattern, an address. Nothing for a tool with
/// nothing to say (an MCP tool's): its name already says what it is.
fn kimi_summary(name: &str, args: Option<&serde_json::Value>, display: Option<&serde_json::Value>) -> Option<String> {
    let arg = |k: &str| args.and_then(|a| a.get(k)).and_then(|v| v.as_str());
    let disp = |k: &str| display.and_then(|d| d.get(k)).and_then(|v| v.as_str());
    match name {
        "Bash" => one_line(arg("command").or(disp("command"))?),
        "Grep" => match (arg("pattern"), arg("path").or(disp("path"))) {
            (Some(pattern), Some(path)) => one_line(&format!("{pattern} in {path}")),
            (Some(pattern), None) => one_line(pattern),
            _ => one_line(disp("path")?),
        },
        "Glob" => one_line(arg("pattern")?),
        "WebSearch" => one_line(arg("query").or(disp("query"))?),
        "FetchURL" => one_line(arg("url").or(disp("url"))?),
        _ => match display.and_then(|d| d.get("kind")).and_then(|k| k.as_str()) {
            Some("file_io") => one_line(arg("path").or(disp("path"))?),
            Some("todo_list") => display.and_then(|d| d.get("items")).and_then(|i| i.as_array()).map(|i| format!("{} {}", i.len(), if i.len() == 1 { "item" } else { "items" })),
            Some("agent_call") => one_line(disp("agent_name")?),
            Some("skill_call") => one_line(disp("skill_name")?),
            _ => None,
        },
    }
}

/// kimi tools whose `args` carry a file's contents: like Claude's
/// [`PATH_ONLY`], only the path is kept, as the contents stay in the terminal.
/// kimi's own names and argument keys, measured on 2.2.0.
const KIMI_PATH_ONLY: [(&str, &str); 2] = [("Write", "path"), ("Edit", "path")];

/// What a kimi tool call was given: the command or the path for a tool
/// [`kimi_summary`] names, only the path of a [`KIMI_PATH_ONLY`] tool (its
/// `content`/`old_string`/`new_string` stay in the terminal), and the
/// structured arguments for any other tool, an MCP tool's included.
fn kimi_ran(name: &str, args: Option<&serde_json::Value>) -> Ran {
    let Some(args) = args else { return Ran::Nothing };
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::to_string);
    if let Some((_, key)) = KIMI_PATH_ONLY.iter().find(|(tool, _)| *tool == name) {
        return s(key).map_or(Ran::Nothing, |path| Ran::Parts(vec![Some(path)]));
    }
    let text = match name {
        "Bash" => s("command"),
        "Read" | "Grep" => match (s("pattern"), s("path")) {
            (Some(pattern), Some(path)) => Some(format!("{pattern} in {path}")),
            (Some(pattern), None) => Some(pattern),
            _ => s("path"),
        },
        "Glob" => s("pattern"),
        "WebSearch" => s("query"),
        "FetchURL" => s("url"),
        _ => return ran_json(args),
    };
    text.map_or(Ran::Nothing, |t| Ran::Parts(vec![Some(t)]))
}

/// The input of the kimi call `id`, as [`claude_tool_call`]. A call a person
/// decided at its approval is a permission step by then, and holds no input,
/// as Claude's model has it.
pub fn kimi_tool_call(lines: &[String], id: &str) -> Option<CallDetail> {
    detail_in(kimi_turns(lines), id)
}

/// The turn a reply belongs to: the last one, or a prompt-less one when the
/// window starts inside a turn.
fn current<'a>(turns: &'a mut Vec<Turn>, at: Option<i64>, id: &str) -> &'a mut Turn {
    if turns.is_empty() {
        let mut turn = Turn::new(at, None);
        turn.id = id.to_string();
        turns.push(turn);
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
            t.truncated |= clip_steps(&mut t.steps, TEXT_MAX, TEXT_MAX);
            mark_detail(&mut t);
            t
        })
        .collect()
}

fn mark_detail(t: &mut Turn) {
    for step in &mut t.steps {
        if let Step::Tool { summary, ran, detail, .. } = step {
            *detail = says_more(ran, summary.as_deref());
        }
    }
}

/// The turn `id` names, whole: read by [`claude_turns`]'s own parser like
/// [`claude`], and cut only at [`TURN_TEXT_MAX`]. `None` once it is not in
/// `lines` (the transcript's window has moved past it) or `id` names no turn.
pub fn claude_turn(lines: &[String], id: &str) -> Option<Turn> {
    whole(claude_turns(lines), id)
}

/// [`claude_turn`] for a Codex rollout's `lines`.
pub fn codex_turn(lines: &[String], id: &str) -> Option<Turn> {
    whole(codex_turns(lines), id)
}

/// One turn at the whole-turn cap. Its `reply` is left empty: the text is in
/// `steps`, and carrying both would send every word twice.
fn whole(turns: Vec<Turn>, id: &str) -> Option<Turn> {
    if id.is_empty() {
        return None;
    }
    let mut t = turns.into_iter().find(|t| t.id == id)?;
    if let Some(u) = t.user.take() {
        let (u, cut) = clip_to(&u, TURN_TEXT_MAX);
        t.user = Some(u);
        t.truncated |= cut;
    }
    t.reply.clear();
    t.truncated |= clip_steps(&mut t.steps, TURN_TEXT_MAX, TURN_TEXT_MAX);
    mark_detail(&mut t);
    Some(t)
}

/// The steps held to the reply's caps: `text` characters of reply text
/// across all `text` steps (any past it go), and `each` for each other
/// string. True when anything was cut.
fn clip_steps(steps: &mut Vec<Step>, text: usize, each: usize) -> bool {
    let mut cut = false;
    let mut left = text;
    steps.retain_mut(|step| {
        let Step::Text { text, .. } = step else { return true };
        if left == 0 {
            cut = true;
            return false;
        }
        let (out, was) = clip_to(text, left);
        left = if was { 0 } else { left - out.chars().count() };
        cut |= was;
        *text = out;
        true
    });
    let mut c = |s: &mut String| {
        let (out, was) = clip_to(s, each);
        *s = out;
        cut |= was;
    };
    for step in steps.iter_mut() {
        match step {
            Step::User { text, .. } => c(text),
            Step::Question { questions, answer, .. } => {
                for q in questions {
                    c(&mut q.header);
                    c(&mut q.question);
                    for o in &mut q.options {
                        c(&mut o.label);
                        c(&mut o.description);
                    }
                }
                for a in answer.iter_mut().flat_map(|a| a.answers.iter_mut()) {
                    c(&mut a.question);
                    a.picked.iter_mut().for_each(&mut c);
                    a.typed.iter_mut().for_each(&mut c);
                    a.note.iter_mut().for_each(&mut c);
                }
            }
            Step::Plan { plan, note, .. } => {
                c(plan);
                note.iter_mut().for_each(&mut c);
            }
            Step::Permission { note, .. } => note.iter_mut().for_each(&mut c),
            _ => {}
        }
    }
    cut
}

fn clip(s: &str) -> (String, bool) {
    clip_to(s, TEXT_MAX)
}

/// `s` cut to `max` characters, an ellipsis marking the cut.
fn clip_to(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    (out, true)
}

/// A tool call's one-line summary, at most this many characters.
const SUMMARY_MAX: usize = 200;

impl Step {
    /// The tool call this step answers to, when it is one.
    fn id(&self) -> Option<&str> {
        match self {
            Step::Tool { id, .. } | Step::Question { id, .. } | Step::Plan { id, .. } if !id.is_empty() => Some(id),
            _ => None,
        }
    }

    /// The text it carries, for [`TOTAL_MAX`].
    fn bytes(&self) -> usize {
        let opt = |s: &Option<String>| s.as_ref().map_or(0, String::len);
        match self {
            Step::Text { text, .. } | Step::User { text, .. } => text.len(),
            Step::Tool { name, id, summary, failed, detail, .. } => {
                name.len() + opt(summary) + if id.is_empty() { 0 } else { CALL_ID_BYTES + id.len() } + if *failed { FAILED_BYTES } else { 0 } + if *detail { DETAIL_BYTES } else { 0 }
            }
            Step::Question { questions, answer, .. } => {
                questions.iter().map(|q| q.header.len() + q.question.len() + q.options.iter().map(|o| o.label.len() + o.description.len()).sum::<usize>()).sum::<usize>()
                    + answer.iter().flat_map(|a| &a.answers).map(|a| a.question.len() + a.picked.iter().map(String::len).sum::<usize>() + opt(&a.typed) + opt(&a.note)).sum::<usize>()
            }
            Step::Permission { tool, summary, scope, note, .. } => tool.len() + opt(summary) + opt(scope) + opt(note),
            Step::Plan { plan, plan_file, note, .. } => plan.len() + opt(plan_file) + opt(note),
        }
    }
}

/// What a tool step's `failed` costs toward [`TOTAL_MAX`]: its JSON.
const FAILED_BYTES: usize = r#","failed":true"#.len();
/// What a tool step's `detail` costs, as `failed` does.
const DETAIL_BYTES: usize = r#","detail":true"#.len();
/// What a tool step's `call_id` costs beyond the id itself.
const CALL_ID_BYTES: usize = r#","call_id":"""#.len();

/// `turns` cut to [`TOTAL_MAX`] bytes of text, newest kept. Whole turns go
/// first, oldest first, counted WITHOUT their tool calls' summaries, so a
/// summary never costs a turn; then the kept turns' summaries, oldest first,
/// until the rest fits — each call keeping its name, its flags and its id, so
/// a call with no line left can still be opened.
/// The flag says whether any turn went. The newest turn always stays.
pub fn within_budget(mut turns: Vec<Turn>) -> (Vec<Turn>, bool) {
    let words = |t: &Turn| t.bytes() - t.summary_bytes();
    let mut dropped = false;
    while turns.len() > 1 && turns.iter().map(words).sum::<usize>() > TOTAL_MAX {
        turns.remove(0);
        dropped = true;
    }
    let mut over = turns.iter().map(Turn::bytes).sum::<usize>().saturating_sub(TOTAL_MAX);
    for step in turns.iter_mut().flat_map(|t| t.steps.iter_mut()) {
        if over == 0 {
            break;
        }
        if let Step::Tool { summary: summary @ Some(_), .. } = step {
            over = over.saturating_sub(summary.take().map_or(0, |s| s.len()));
        }
    }
    (turns, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    /// The turns as the wire has them, less each tool step's `call_id`: the
    /// one place a call's id may appear, so a test can look for it elsewhere.
    fn without_call_ids(turns: &[Turn]) -> String {
        fn strip(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Object(o) => {
                    o.remove("call_id");
                    o.values_mut().for_each(strip);
                }
                serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
                _ => {}
            }
        }
        let mut v = serde_json::to_value(turns).unwrap();
        strip(&mut v);
        v.to_string()
    }

    #[test]
    fn claude_injected_entries_are_not_person_messages() {
        use serde_json::json;
        let mut fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.316-injected.jsonl"));
        fx.push(json!({"type":"user","isMeta":true,"message":{"role":"user","content":"META-ONLY-MARKER"}}).to_string());
        fx.push(json!({"type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"<task-notification><status>completed</status></task-notification>"}}).to_string());
        fx.push(json!({"type":"user","message":{"role":"user","content":"I saw <task-notification><status>completed</status></task-notification> in the log; what is it?"}}).to_string());
        let got = claude(&fx, 20);
        let values: Vec<serde_json::Value> = got.iter().map(|t| serde_json::to_value(t).unwrap()).collect();
        let kinds: Vec<&str> = values.iter().filter_map(|t| t.pointer("/notice/kind").and_then(|v| v.as_str())).collect();
        for kind in ["task", "peer_message", "command", "shell", "interrupt", "summary"] {
            assert!(kinds.contains(&kind), "missing {kind}: {kinds:?}");
        }
        assert!(values.iter().filter(|t| t.get("notice").is_some()).all(|t| t["user"].as_str().is_some_and(|s| !s.contains("<task-notification>"))));
        assert_eq!(values[values.len() - 2]["user"], "<task-notification><status>completed</status></task-notification>");
        assert!(values[values.len() - 2].get("notice").is_none(), "human origin must win");
        assert_eq!(values.last().unwrap()["user"], "I saw <task-notification><status>completed</status></task-notification> in the log; what is it?");
        assert!(values.last().unwrap().get("notice").is_none(), "prose mentioning a wrapper stays a user message");
        let all = serde_json::to_string(&values).unwrap();
        assert!(!all.contains("META-ONLY-MARKER"), "isMeta entry leaked");
        assert!(!all.contains("SYSTEM-PROMPT-MARKER"), "system reminder leaked");
        assert!(!all.contains("<task-notification>\\n"), "raw wrapper leaked");
    }

    /// Claude 2.1.318 writes a local command's name and output as `system` /
    /// `local_command` entries, with ANSI in the output, and a prompt
    /// command's `human` entry with `<command-message>` before
    /// `<command-name>` (a real, trimmed transcript). Each is the command's
    /// notice: its output keeps its lines and loses its escapes, and nothing
    /// Claude wrote for the model (the markdown copy, the expansion, the
    /// compact summary) comes back.
    #[test]
    fn claude_2_1_318_commands_are_notices() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.318-commands.jsonl"));
        let got = claude(&fx, 20);
        let all = serde_json::to_string(&got).unwrap();
        for secret in ["CONTEXT-META-MARKER", "COMMAND-EXPANSION-MARKER", "REMINDER-MARKER", "THINKING-MARKER", "TOOL-OUTPUT-MARKER", "<command-", "local-command", "\\u001b", "[1m", "[22m"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        let commands: Vec<(&str, &str, Option<&str>)> = got
            .iter()
            .filter_map(|t| match &t.notice {
                Some(Notice::Command { name, args, output }) => Some((name.as_str(), args.as_str(), output.as_deref())),
                _ => None,
            })
            .collect();
        let names: Vec<(&str, &str)> = commands.iter().map(|(n, a, _)| (*n, *a)).collect();
        assert_eq!(
            names,
            vec![
                ("context", ""),
                ("model", ""),
                ("context", ""),
                ("compact", "keep only that we listed animal facts"),
                ("clear", ""),
                ("hello", "world"),
                ("preexec", ""),
                ("model", ""),
                ("context", ""),
                ("context", ""),
            ]
        );
        assert!(got.iter().any(|t| matches!(&t.notice, Some(Notice::Summary { text }) if text.contains("SUMMARY-MARKER"))), "the summary, its reminder cut");
        let context = commands[0].2.expect("/context's output");
        assert!(context.starts_with("Context Usage\n"), "{context:?}");
        assert!(context.lines().count() > 10 && context.contains("27.6k/1m tokens (3%)"), "{context:?}");
        assert_eq!(commands[1].2, Some("Set model to `Opus 5.5` for this session only"));
        assert_eq!(commands[3].2, Some("Compacted (ctrl+o to see full summary)"));
        assert_eq!(commands[4].2, None, "an empty output is none");
        let reply = |name: &str| got.iter().find(|t| matches!(&t.notice, Some(Notice::Command { name: n, .. }) if n == name)).map(|t| t.reply.as_str());
        assert_eq!(reply("hello"), Some("Hello, world!"), "a prompt command's reply is its turn's");
        assert_eq!(reply("preexec"), Some("ok"));
        assert!(got.iter().any(|t| t.user.as_deref() == Some("Reply with the single word pong. Use no tools.") && t.reply == "pong"));
        // The literal line /compact writes first is the command's turn, once.
        assert!(!got.iter().any(|t| t.notice.is_none() && t.user.as_deref().is_some_and(|u| u.starts_with("/compact"))), "{got:#?}");
    }

    /// A Claude transcript shaped like 2.1.309's (the entry types and fields a
    /// real session wrote, the content invented): user prompts, replies and
    /// structured local-command and interrupt notices come back, without
    /// system, attachment, sidechain or raw wrapper content.
    #[test]
    fn claude_turns_are_prompts_and_replies_only() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.309.jsonl"));
        let got = claude(&fx, 10);
        let all = serde_json::to_string(&got).unwrap();
        for secret in ["SYSTEM-PROMPT-MARKER", "CLAUDE-MD-MARKER", "ATTACHMENT-MARKER", "SIDECHAIN-MARKER", "THINKING-MARKER", "TOOL-OUTPUT-MARKER", "local-command"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        assert_eq!(got.len(), 5, "{got:#?}");
        assert_eq!(got[0].user.as_deref(), Some("Use the Bash tool to run exactly: touch probe-a.txt"));
        assert_eq!(got[0].tools, vec!["Bash".to_string()]);
        assert_eq!(got[0].reply, "Done.");
        assert_eq!(got[0].at, crate::sysclock::parse_iso8601("2026-10-23T18:48:01.000Z"));
        assert!(matches!(got[1].notice, Some(Notice::Command { .. })));
        assert_eq!(got[2].tools, vec!["AskUserQuestion".to_string()]);
        assert_eq!(got[2].reply, "Blue");
        assert_eq!(got[3].reply, "", "a rejected tool call answers nothing");
        assert!(matches!(got[4].notice, Some(Notice::Interrupt)));
        assert_eq!(claude(&fx, 1).len(), 1);
        assert_eq!(claude(&fx, 1)[0].notice, got[4].notice, "the LAST turns");
        // A window that starts mid-turn has a reply with no prompt.
        let tail: Vec<String> = fx.iter().skip_while(|l| !l.contains("\"Done.\"")).cloned().collect();
        let got = claude(&tail, 10);
        assert_eq!((got[0].user.as_deref(), got[0].reply.as_str()), (None, "Done."));
    }

    fn ts(s: &str) -> i64 {
        crate::sysclock::parse_iso8601(s).unwrap()
    }

    fn claude_bash(id: &str, command: &str) -> Vec<String> {
        use serde_json::json;
        vec![
            json!({"type":"user","message":{"role":"user","content":"Run it."}}).to_string(),
            tool_use(id, "Bash", json!({"command":command})),
        ]
    }

    fn tool_use(id: &str, name: &str, input: serde_json::Value) -> String {
        serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}).to_string()
    }

    /// (summary, call_id, detail) of each tool step, as the wire has them.
    fn rows(turns: &[Turn]) -> Vec<(Option<String>, Option<String>, bool)> {
        let v = serde_json::to_value(turns).unwrap();
        v.as_array().unwrap().iter().flat_map(|t| t["steps"].as_array().cloned().unwrap_or_default()).filter(|s| s["kind"] == "tool").map(|s| {
            (s["summary"].as_str().map(str::to_string), s["call_id"].as_str().map(str::to_string), s["detail"] == true)
        }).collect()
    }

    fn input_of(c: Option<CallDetail>) -> Option<String> {
        c.map(|c| c.input.as_str().expect("a string input").to_string())
    }

    /// A multi-line command comes back whole, its shape kept and its summary
    /// the one line it always was; `detail` says only when there is more.
    #[test]
    fn claude_call_is_the_whole_command_and_the_summary_is_unchanged() {
        let command = "set -e\n\tcargo test \\\n  -p core\r\necho done\rtail";
        let fx = claude_bash("toolu_a", command);
        assert_eq!(rows(&claude(&fx, 5)), vec![(Some("set -e".to_string()), Some("toolu_a".to_string()), true)]);
        let got = claude_tool_call(&fx, "toolu_a").unwrap();
        assert_eq!(got.tool, "Bash");
        assert_eq!(got.input, "set -e\n\tcargo test \\\n  -p core\necho done\ntail");
        assert!(!got.truncated);
        assert_eq!(claude_tool_call(&fx, "toolu_other"), None, "a call that left the window is nothing");
        assert_eq!(claude_tool_call(&fx, ""), None);
        // One short line: nothing more to show, tab or not. Its id still rides.
        for short in ["ls -la", "ls\t-la", "\n  ls -la\n\n"] {
            let got = rows(&claude(&claude_bash("toolu_b", short), 5));
            assert_eq!(got[0].1.as_deref(), Some("toolu_b"), "{short:?}");
            assert!(!got[0].2, "{short:?}");
        }
        // A line past the summary's clip does have more.
        let long = "x".repeat(SUMMARY_MAX + 1);
        let fx = claude_bash("toolu_c", &long);
        assert!(rows(&claude(&fx, 5))[0].2);
        assert_eq!(input_of(claude_tool_call(&fx, "toolu_c")).unwrap(), long);
    }

    /// What a person reads is what ran: an escape's byte and a bidi override
    /// are gone, from a command and inside a structured input's strings, on
    /// both harnesses. Nothing is redacted: the token stays.
    #[test]
    fn a_call_input_loses_control_and_invisible_characters_only() {
        use serde_json::json;
        let raw = "printf '\u{1b}[31mred'\nrm \u{202e}txt.\u{202c} a\u{200b}b\u{7} TOKEN=abc123";
        let want = "printf '[31mred'\nrm txt. ab TOKEN=abc123";
        assert_eq!(input_of(claude_tool_call(&claude_bash("c", raw), "c")).unwrap(), want);
        let script = format!("await tools.exec_command({{cmd: {}}});", serde_json::to_string(raw).unwrap());
        let fx = vec![
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"go"}]}}).to_string(),
            json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","call_id":"x","input":script}}).to_string(),
        ];
        assert_eq!(input_of(codex_tool_call(&fx, "x")).unwrap(), want);
        let fx = vec![tool_use("m", "mcp__a__b", json!({"q\u{200b}":[{"k":raw}],"n":1}))];
        assert_eq!(claude_tool_call(&fx, "m").unwrap().input, json!({"q":[{"k":want}],"n":1}));
    }

    /// Past the cap a command is cut on a character, and a structured input
    /// comes back as text rather than as JSON that does not parse.
    #[test]
    fn a_call_input_is_capped_and_says_so() {
        use serde_json::json;
        let long = "é".repeat(CALL_INPUT_MAX); // two bytes each
        let got = claude_tool_call(&claude_bash("c", &long), "c").unwrap();
        assert!(got.truncated);
        assert_eq!(got.bytes, CALL_INPUT_MAX * 2);
        assert_eq!(got.input.as_str().unwrap().len(), CALL_INPUT_MAX);
        let at_cap = claude_tool_call(&claude_bash("c", &"a".repeat(CALL_INPUT_MAX)), "c").unwrap();
        assert!(!at_cap.truncated);

        let got = claude_tool_call(&[tool_use("m", "mcp__a__b", json!({"body":"x".repeat(CALL_INPUT_MAX)}))], "m").unwrap();
        assert!(got.truncated);
        assert!(got.bytes > CALL_INPUT_MAX);
        let text = got.input.as_str().expect("a cut value is a string");
        assert_eq!(text.len(), CALL_INPUT_MAX);
        assert!(text.starts_with("{\n  \"body\": \"xxx"), "the value's own text");
        assert!(serde_json::from_str::<serde_json::Value>(text).is_err());
    }

    /// Only a `tool_use` in an assistant entry is ever read: a request blob
    /// and a sidechain naming the same id hand back nothing.
    #[test]
    fn a_claude_call_reads_only_assistant_tool_use() {
        use serde_json::json;
        let mut sidechain: serde_json::Value = serde_json::from_str(&tool_use("side", "Bash", json!({"command":"SIDECHAIN-MARKER\nb"}))).unwrap();
        sidechain["isSidechain"] = json!(true);
        let fx = vec![
            json!({"type":"user","message":{"role":"user","content":"Go."}}).to_string(),
            json!({"type":"api-request-blob","message":{"role":"assistant","content":[{"type":"tool_use","id":"blob","name":"Bash","input":{"command":"SYSTEM-PROMPT-MARKER\nb"}}]}}).to_string(),
            sidechain.to_string(),
            tool_use("agent", "Agent", json!({"description":"Look around","prompt":"Read the tree.\nReport back."})),
            tool_use("grep", "Grep", json!({"pattern":"a\nb","path":"/r"})),
        ];
        assert_eq!(claude_tool_call(&fx, "blob"), None);
        assert_eq!(claude_tool_call(&fx, "side"), None);
        assert_eq!(input_of(claude_tool_call(&fx, "agent")).unwrap(), "Read the tree.\nReport back.");
        assert_eq!(input_of(claude_tool_call(&fx, "grep")).unwrap(), "a\nb in /r");
        let sent = serde_json::to_string(&claude(&fx, 5)).unwrap();
        assert!(!sent.contains("Report back"), "the input never rides on the turns: {sent}");
        assert!(!sent.contains("\"ran\""), "{sent}");
    }

    /// The named exclusion: however a file-writing tool's input is shaped,
    /// only its path leaves, and `detail` says so only for a path its line
    /// cut. The generic input path would hand back every one of these strings.
    #[test]
    fn a_file_writing_tools_contents_never_come_back() {
        use serde_json::json;
        let path = format!("/r/{}", "d/".repeat(SUMMARY_MAX));
        let fx = vec![
            tool_use("edit", "Edit", json!({"file_path":path,"old_string":"LEAK","new_string":"LEAK","replace_all":true})),
            tool_use("multi", "MultiEdit", json!({"file_path":path,"edits":[{"old_string":"LEAK","new_string":"LEAK"}]})),
            tool_use("write", "Write", json!({"file_path":path,"content":"LEAK"})),
            tool_use("note", "NotebookEdit", json!({"notebook_path":path,"new_source":"LEAK","cell_id":"LEAK"})),
        ];
        for id in ["edit", "multi", "write", "note"] {
            let got = claude_tool_call(&fx, id).unwrap();
            assert_eq!(got.input, json!(path), "{id}");
            assert!(!serde_json::to_string(&got).unwrap().contains("LEAK"), "{id}");
        }
        let got = claude(&fx, 5);
        assert!(rows(&got).iter().all(|r| r.2 && r.1.is_some() && r.0.is_some()), "a long path has more than its line: {got:#?}");
        assert!(!serde_json::to_string(&got).unwrap().contains("LEAK"));
        assert_eq!(PATH_ONLY.map(|(tool, _)| tool), ["Edit", "MultiEdit", "Write", "NotebookEdit"]);
        // A path that fits has nothing more to show; no path, nothing at all.
        let fx = vec![
            tool_use("short", "Write", json!({"file_path":"/r/a.rs","content":"LEAK"})),
            tool_use("none", "Write", json!({"content":"LEAK"})),
        ];
        assert!(rows(&claude(&fx, 5)).iter().all(|r| !r.2));
        assert_eq!(claude_tool_call(&fx, "none"), None);
    }

    /// Any other tool, MCP's and an unknown one's, answers with its input as
    /// the structured value it is. An empty one has nothing to open.
    #[test]
    fn an_mcp_call_is_its_structured_input() {
        use serde_json::json;
        let fx = vec![
            tool_use("m", "mcp__tavily__search", json!({"query":"a\nb","opts":[{"key":1}]})),
            tool_use("empty", "mcp__x__ping", json!({})),
            tool_use("todo", "TodoWrite", json!({"todos":[{"content":"one"}]})),
        ];
        let got = claude_tool_call(&fx, "m").unwrap();
        assert_eq!(got.tool, "mcp__tavily__search");
        assert_eq!(got.input, json!({"query":"a\nb","opts":[{"key":1}]}));
        assert_eq!(claude_tool_call(&fx, "empty"), None);
        let turns = claude(&fx, 5);
        let got = rows(&turns);
        assert_eq!(got[0], (None, Some("m".to_string()), true));
        assert_eq!(got[1], (None, Some("empty".to_string()), false));
        assert_eq!(got[2], (Some("1 item".to_string()), Some("todo".to_string()), true));
        assert!(!serde_json::to_string(&turns).unwrap().contains("tavily__search\",\"input"));
        // Codex: a function call's JSON arguments, or a custom tool's text.
        let cx = vec![
            json!({"type":"response_item","payload":{"type":"function_call","name":"mcp__x__y","call_id":"f","arguments":"{\"q\":\"a\\u001bb\"}"}}).to_string(),
            json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"mine","call_id":"t","input":"raw\ntext"}}).to_string(),
            json!({"type":"response_item","payload":{"type":"function_call","name":"bad","call_id":"b","arguments":"not json"}}).to_string(),
        ];
        assert_eq!(codex_tool_call(&cx, "f").unwrap().input, json!({"q":"ab"}));
        assert_eq!(codex_tool_call(&cx, "t").unwrap().input, "raw\ntext");
        assert_eq!(codex_tool_call(&cx, "b").unwrap().input, "not json");
    }

    /// An input that says only what its line says has nothing to open.
    #[test]
    fn an_input_that_repeats_its_line_has_no_detail() {
        use serde_json::json;
        let fx = vec![
            tool_use("same", "Skill", json!({"skill":"close-out"})),
            tool_use("more", "Skill", json!({"skill":"close-out","args":"--dry-run"})),
            tool_use("todo", "TodoWrite", json!({"todos":[]})),
            tool_use("one", "mcp__a__b", json!({"q":"x"})),
        ];
        let got = rows(&claude(&fx, 5));
        assert_eq!(got[0], (Some("close-out".to_string()), Some("same".to_string()), false));
        assert!(got[1].2, "{got:?}");
        assert!(!got[2].2, "an input with no values says nothing: {got:?}");
        assert!(got[3].2, "no line to repeat: {got:?}");
    }

    /// Two keys that differ only by what cleaning removes must not lose a
    /// value: a hidden character could otherwise hide one from the reader.
    #[test]
    fn keys_that_clean_alike_keep_both_values() {
        use serde_json::json;
        let fx = vec![tool_use("m", "mcp__a__b", json!({"a":1,"a\u{200b}":2,"a\u{202e}":3,"a (2)":4}))];
        let got = claude_tool_call(&fx, "m").unwrap().input;
        let o = got.as_object().unwrap();
        assert_eq!(o.len(), 4, "{got}");
        let mut values: Vec<i64> = o.values().map(|v| v.as_i64().unwrap()).collect();
        values.sort();
        assert_eq!(values, [1, 2, 3, 4], "{got}");
        assert_eq!(o["a"], 1);
        assert_eq!(o["a (2)"], 4, "a key the input really has keeps its value: {got}");
    }

    /// The budget counts what a step's id and flags cost on the wire.
    #[test]
    fn a_tool_steps_bytes_count_its_id_and_detail() {
        use serde_json::json;
        let got = claude(&[tool_use("id1", "mcp__a__b", json!({"q":"x"}))], 5);
        let step = &got[0].steps[0];
        let wire = serde_json::to_string(step).unwrap();
        assert!(wire.contains(r#","call_id":"id1""#) && wire.contains(r#","detail":true"#), "{wire}");
        assert_eq!(step.bytes(), "mcp__a__b".len() + r#","call_id":"id1""#.len() + r#","detail":true"#.len());
    }

    /// Every shape Codex's summary reads gives its whole text too: a script's
    /// commands in order, a patch's files and never its text, both argv forms
    /// and a command only an event names.
    #[test]
    fn codex_call_is_every_command_in_each_shape() {
        use serde_json::json;
        let user = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"go"}]}}).to_string();
        let exec = |id: &str, js: &str| json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","call_id":id,"input":js}}).to_string();
        let func = |id: &str, name: &str, args: serde_json::Value| json!({"type":"response_item","payload":{"type":"function_call","name":name,"call_id":id,"arguments":args.to_string()}}).to_string();
        let done = |id: &str| json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":id,"output":"OUTPUT-MARKER"}}).to_string();
        let event = |cmd: &str| json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","command":["/bin/zsh","-lc",cmd],"status":"completed","exit_code":0}}}).to_string();
        let patch = "*** Begin Patch\n*** Update File: a.rs\n+PATCH-MARKER\n*** Add File: b.rs\n+PATCH-MARKER\n*** End Patch";
        let fx = vec![
            user,
            exec("two", "await tools.exec_command({cmd: \"set -e\\nls\"}); await tools.exec_command({cmd: 'make test'});"),
            done("two"),
            exec("mix", &format!("await tools.apply_patch({}); await tools.exec_command({{cmd: \"cat a.rs\"}}); await tools.view_image({{path: \"a).png\"}});", serde_json::to_string(patch).unwrap())),
            done("mix"),
            func("cmd", "exec_command", json!({"cmd":"git status\ngit diff"})),
            done("cmd"),
            func("argv", "shell", json!({"command":["/bin/bash","-lc","cd x\nmake"]})),
            done("argv"),
            func("words", "shell", json!({"command":["ls","-l","src"]})),
            done("words"),
            func("legacy", "apply_patch", json!({"input":patch})),
            done("legacy"),
            exec("built", "const c = ['ls'].join(' '); await tools.exec_command({cmd: c}); await tools.exec_command({cmd: \"pwd\"}); await tools.exec_command({cmd: `a${c}`});"),
            event("pwd"),
            event("ls -la\nwc -l"),
            done("built"),
            exec("nobody", "const c = 'x'; await tools.exec_command({cmd: c});"),
            done("nobody"),
        ];
        let text = |id: &str| input_of(codex_tool_call(&fx, id));
        assert_eq!(text("two").as_deref(), Some("set -e\nls\n\nmake test"));
        assert_eq!(text("mix").as_deref(), Some("edit a.rs; add b.rs\n\ncat a.rs\n\nview_image({path: \"a).png\"})"));
        assert_eq!(text("cmd").as_deref(), Some("git status\ngit diff"));
        assert_eq!(text("argv").as_deref(), Some("cd x\nmake"));
        assert_eq!(text("words").as_deref(), Some("ls -l src"));
        assert_eq!(text("legacy").as_deref(), Some("edit a.rs; add b.rs"));
        assert_eq!(text("built").as_deref(), Some("ls -la\nwc -l\n\npwd\n\n(a command this script builds)"));
        assert_eq!(text("nobody"), None, "nothing is known of it: no click");
        let turns = codex(&fx, 5);
        let sent = serde_json::to_string(&turns).unwrap();
        for leak in ["PATCH-MARKER", "OUTPUT-MARKER", "make test"] {
            assert!(!sent.contains(leak), "{leak}: {sent}");
        }
        let got = rows(&turns);
        let row = |sum: Option<&str>, id: &str, more: bool| (sum.map(str::to_string), Some(id.to_string()), more);
        assert_eq!(got[0], row(Some("set -e (+1 more)"), "two", true));
        assert_eq!(got[2], row(Some("git status"), "cmd", true));
        assert_eq!(got[4], row(Some("ls -l src"), "words", false), "one short line");
        assert_eq!(got[5], row(Some("edit a.rs; add b.rs"), "legacy", false), "a patch's list that fits");
        assert!(got[6].2);
        assert_eq!(got[7], row(None, "nobody", false));
    }

    /// The budget takes a call's line and leaves its id and `detail`: the call
    /// can still be opened, which is when its text is worth anything.
    #[test]
    fn the_budget_leaves_a_call_its_id_and_detail() {
        use serde_json::json;
        let long = format!("{}\nsecond", "c".repeat(190));
        let mut fx = vec![json!({"type":"user","message":{"role":"user","content":"Go."}}).to_string()];
        for i in 0..400 {
            fx.push(claude_bash(&format!("toolu_{i:03}"), &long).remove(1));
        }
        let (kept, dropped) = within_budget(claude(&fx, 1));
        assert!(!dropped);
        let got = rows(&kept);
        assert!(got.iter().any(|r| r.0.is_none()), "the fixture must be over budget");
        assert!(got.iter().all(|r| r.1.is_some() && r.2), "{:?}", got.iter().find(|r| r.1.is_none() || !r.2));
        assert!(kept[0].bytes() <= TOTAL_MAX);
    }

    #[test]
    fn claude_queued_message_is_a_step_only_when_absorbed() {
        use serde_json::json;
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.316-queued-synthetic.jsonl"));
        let got = claude(&fx, 20);
        assert_eq!(got.len(), 3, "{got:#?}");
        assert_eq!(got[0].at, Some(ts("2026-11-01T00:44:50.000Z")));
        assert_eq!(got[0].user.as_deref(), Some("Start the tile."));
        assert_eq!(
            serde_json::to_value(&got[0].steps).unwrap(),
            json!([
                {"kind":"text", "at":ts("2026-11-01T00:45:00.000Z"), "text":"Working on the tile."},
                {"kind":"user", "at":ts("2026-11-01T00:45:10.916Z"), "text":"Layer these images [attached: .worktrees-attachments/20261102-aaaaaaaaaaaaaaaa.png, .worktrees-attachments/20261102-bbbbbbbbbbbbbbbb.png]"},
                {"kind":"text", "at":ts("2026-11-01T00:45:20.000Z"), "text":"I layered them."}
            ])
        );
        assert_eq!(got[1].user.as_deref(), Some("Next, inspect the colors."));
        assert!(serde_json::to_string(&got[1].steps).unwrap().find("\"kind\":\"user\"").is_none(), "delivered queue has one normal turn");
        assert_eq!(got[2].at, Some(ts("2026-11-01T05:00:47.606Z")), "an unfinished normal turn keeps its prompt time");
        assert_eq!(got[2].user.as_deref(), Some("Give me more options."));
        assert!(!serde_json::to_string(&got).unwrap().contains("RENDERED-REMINDER-MARKER"));
    }

    #[test]
    fn a_queued_human_step_removes_invisible_and_control_characters() {
        use serde_json::json;
        let fx = [
            json!({"type":"user", "message":{"content":"Start"}}),
            json!({"type":"attachment", "attachment":{"type":"queued_command", "prompt":"one\u{202e}two\u{1b}three", "source_uuid":"s1", "delivery_id":"d1", "commandMode":"prompt", "humanTurn":true}}),
            json!({"type":"queue-operation", "operation":"remove", "reason":"absorbed_mid_turn", "commandUuid":"s1", "deliveryId":"d1"}),
            json!({"type":"attachment", "attachment":{"type":"queued_command", "prompt":"SYSTEM-MARKER", "source_uuid":"s2", "delivery_id":"d2", "commandMode":"prompt", "origin":{"kind":"system"}}}),
            json!({"type":"queue-operation", "operation":"remove", "reason":"absorbed_mid_turn", "commandUuid":"s2", "deliveryId":"d2"}),
        ];
        let lines: Vec<String> = fx.iter().map(serde_json::Value::to_string).collect();
        let got = claude(&lines, 1);
        assert_eq!(serde_json::to_value(&got[0].steps).unwrap(), json!([{"kind":"user", "at":null, "text":"onetwothree"}]));
        assert!(!serde_json::to_string(&got).unwrap().contains("SYSTEM-MARKER"));
    }

    /// A real Claude 2.1.316 session, trimmed (README): every question,
    /// permission and plan it was asked comes back as a step, in order,
    /// with the answer and its own time, and `reply` / `tools` stay as an
    /// older client reads them.
    #[test]
    fn claude_steps_carry_what_was_asked_and_answered() {
        use serde_json::json;
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.316-answers.jsonl"));
        let got = claude(&fx, 20);
        let all = without_call_ids(&got);
        for secret in ["SYSTEM-PROMPT-MARKER", "THINKING-MARKER", "noOutputExpected", "drwxr-xr-x", "permissionDecision", "toolu_"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        assert_eq!(got.len(), 8, "{got:#?}");
        assert!(matches!(got[7].notice, Some(Notice::Interrupt)));
        let steps = |i: usize| serde_json::to_value(&got[i].steps).unwrap();

        // Picked from its options; the reply after it has the reply's time.
        let fruit = steps(0);
        assert_eq!(fruit[0]["kind"], "question");
        assert_eq!(fruit[0]["at"], ts("2026-10-31T18:26:03.531Z"));
        assert_eq!(fruit[0]["answered_at"], ts("2026-10-31T18:26:08.726Z"));
        assert_eq!(fruit[0]["questions"][0]["header"], "Fruit");
        assert_eq!(fruit[0]["questions"][0]["question"], "Which fruit should I write a haiku about?");
        assert_eq!(fruit[0]["questions"][0]["multi"], false);
        assert_eq!(fruit[0]["questions"][0]["options"].as_array().unwrap().len(), 3);
        assert!(fruit[0]["questions"][0]["options"][1]["description"].as_str().unwrap().contains("tropical"));
        assert_eq!(fruit[0]["answer"], json!({"how": "picked", "answers": [{"question": "Which fruit should I write a haiku about?", "picked": ["Banana"]}]}));
        assert_eq!(fruit[1]["kind"], "text");
        assert_eq!(fruit[1]["at"], ts("2026-10-31T18:26:10.906Z"));
        assert_eq!(fruit.as_array().unwrap().len(), 2);
        assert_eq!((got[0].tools.clone(), got[0].reply.starts_with("Yellow curve")), (vec!["AskUserQuestion".to_string()], true));

        // Typed in "Type something.", a multi-select, "Chat about this".
        assert_eq!(steps(1)[0]["answer"]["answers"][0], json!({"question": "Which animal should the next haiku be about?", "picked": [], "typed": "a sleepy otter"}));
        assert_eq!(steps(2)[0]["questions"][0]["multi"], true);
        assert_eq!(steps(2)[0]["answer"]["answers"][0]["picked"], json!(["Cheese", "Mushrooms"]));
        assert_eq!(steps(3)[0]["answer"], json!({"how": "chat"}));

        // A Yes; a No with its reason, the retry's Yes with a next step, and
        // a call nobody was asked about (`config`) stays a tool.
        assert_eq!(
            steps(4)[0],
            json!({"kind": "permission", "at": ts("2026-10-31T18:27:39.450Z"), "answered_at": ts("2026-10-31T18:27:40.147Z"), "tool": "Bash", "summary": "touch probe-a.txt", "allowed": true, "scope": "once"})
        );
        let b = steps(5);
        assert_eq!((b[0]["allowed"].clone(), b[0]["note"].clone(), b[0]["summary"].clone()), (json!(false), json!("name it probe-c.txt instead"), json!("touch probe-b.txt")));
        assert_eq!((b[1]["allowed"].clone(), b[1]["note"].clone()), (json!(true), json!("then list the directory")));
        assert_eq!(b[2], json!({"kind": "tool", "at": ts("2026-10-31T18:28:15.685Z"), "name": "Bash", "call_id": "toolu_01KBZkB4A27Gdj3XiCrtEXEU", "summary": "ls -la"}));
        assert_eq!(b[3]["kind"], "text");
        assert_eq!(got[5].tools, vec!["Bash"; 3]);

        // A plan sent back with feedback, then approved; then a refused Write.
        let plan = steps(6);
        let kinds: Vec<&str> = plan.as_array().unwrap().iter().map(|s| s["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["tool", "tool", "plan", "tool", "plan", "permission", "permission"]);
        assert!(plan[2]["plan"].as_str().unwrap().starts_with("# Plan: add hello.sh"));
        assert_eq!((plan[2]["approved"].clone(), plan[2]["note"].clone()), (json!(false), json!("call it greet.sh instead")));
        assert!(plan[4]["plan"].as_str().unwrap().starts_with("# Plan: add greet.sh"));
        assert_eq!((plan[4]["approved"].clone(), plan[4]["plan_file"].clone()), (json!(true), json!("/Users/u/.claude/plans/validated-napping-dream.md")));
        assert_eq!((plan[5]["tool"].clone(), plan[5]["allowed"].clone(), plan[5]["summary"].clone()), (json!("Write"), json!(false), json!("/private/tmp/claude-501/wtask/repo/greet.sh")));
        assert!(plan[5].get("note").is_none());
    }

    /// A real 2.1.316 session of questions with previews (README): a note
    /// typed in the question's notes field rides with the option picked, and
    /// one sent before any option was moved to is the note alone — never the
    /// words `(notes only)` as if typed. "Chat about this" is the chat still.
    #[test]
    fn a_preview_questions_note_comes_back_with_its_answer() {
        use serde_json::json;
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.316-preview-answers.jsonl"));
        let got = claude(&fx, 20);
        let all = serde_json::to_string(&got).unwrap();
        for secret in ["SYSTEM-PROMPT-MARKER", "THINKING-MARKER", "toolu_"] {
            assert!(!all.contains(secret), "{secret} leaked");
        }
        let asked: Vec<serde_json::Value> = got
            .iter()
            .flat_map(|t| serde_json::to_value(&t.steps).unwrap().as_array().unwrap().clone())
            .filter(|s| s["kind"] == "question")
            .collect();
        let banner = "On a place's screen the banner says 1 place needs input, which reads as if it means this place. What should it say instead?";
        assert_eq!(asked[0]["answer"], json!({"how": "picked", "answers": [{"question": banner, "picked": ["Name first"], "note": "keep the place name bold"}]}));
        assert_eq!(asked[1]["answer"], json!({"how": "chat"}));
        assert_eq!(asked[2]["answer"], json!({"how": "declined"}));
        assert_eq!(
            asked[3]["answer"]["answers"],
            json!([{"question": "Which colour?", "picked": [], "note": "q1 note"}, {"question": "Which size?", "picked": ["Large"]}])
        );
        assert_eq!(asked[5]["answer"]["answers"][0], json!({"question": banner, "picked": [], "note": "t1 note"}));
        assert_eq!(asked[6]["answer"]["answers"][0], json!({"question": banner, "picked": ["Another place needs input (Recommended)"], "note": "t2 note"}));
        // Moved away and back first, then the note: the option and the note.
        assert_eq!(asked[9]["answer"]["answers"][0], json!({"question": banner, "picked": ["Another place needs input (Recommended)"], "note": "t4 note"}));
        assert_eq!(asked[10]["answer"]["answers"][0], json!({"question": banner, "picked": ["Another place needs input (Recommended)"]}), "no note, none given");
        assert!(asked[11].get("answer").is_none(), "the last one was still up");
    }

    /// Codex's rollout: the reply's blocks and the tools, in order, each at
    /// its own time; nothing there is a question or an approval.
    #[test]
    fn codex_steps_are_the_reply_and_the_tools_in_order() {
        use serde_json::json;
        let fx = lines(include_str!("../tests/fixtures/transcripts/codex-rollout.jsonl"));
        let got = codex(&fx, 10);
        assert_eq!(
            serde_json::to_value(&got[0].steps).unwrap(),
            json!([
                {"kind": "text", "at": ts("2026-10-23T18:01:02.000Z"), "text": "Running them."},
                {"kind": "tool", "at": ts("2026-10-23T18:01:02.000Z"), "name": "exec_command", "call_id": "c1", "summary": "make test"},
                {"kind": "tool", "at": ts("2026-10-23T18:01:10.000Z"), "name": "apply_patch", "call_id": "c2"},
                {"kind": "text", "at": ts("2026-10-23T18:01:12.000Z"), "text": "All green."},
            ])
        );
    }

    /// A real Codex 0.160.1 rollout, trimmed (README): its plan-mode
    /// `request_user_input` and the answer, with the note given beside it,
    /// come back as a question step. Its approvals do not: the rollout keeps
    /// the escalated call and its output, never the Yes or the No.
    #[test]
    fn codex_steps_carry_a_question_and_its_answer() {
        use serde_json::json;
        let fx = lines(include_str!("../tests/fixtures/transcripts/codex-0.160.1-answers.jsonl"));
        let got = codex(&fx, 10);
        assert!(!serde_json::to_string(&got).unwrap().contains("BASE-INSTRUCTIONS-MARKER"));
        assert_eq!(got.len(), 4, "{got:#?}");
        let ask = serde_json::to_value(&got[3].steps).unwrap();
        assert_eq!(ask[0]["kind"], "question");
        assert_eq!(ask[0]["questions"][0]["header"], "Button color");
        assert_eq!(ask[0]["questions"][0]["options"][1], json!({"label": "Green", "description": "A positive, energetic look."}));
        assert_eq!(ask[0]["answer"], json!({"how": "picked", "answers": [{"question": "Which color should the button be?", "picked": ["Green"], "note": "a darker green please"}]}));
        assert!(ask[0]["answered_at"].as_i64().unwrap() > ask[0]["at"].as_i64().unwrap());
        assert_eq!(ask[1]["text"], "Darker green selected.");
        // An approval is a tool, whatever was answered.
        let kinds: Vec<String> = got[0..3].iter().flat_map(|t| t.steps.iter()).map(|s| serde_json::to_value(s).unwrap()["kind"].as_str().unwrap().to_string()).collect();
        assert!(kinds.iter().all(|k| k == "text" || k == "tool"), "{kinds:?}");
    }

    /// The steps' text is held to the same caps as the reply: TEXT_MAX
    /// characters of reply text in all, and every step counts toward
    /// TOTAL_MAX.
    #[test]
    fn steps_are_held_to_the_text_caps() {
        let long = "y".repeat(TEXT_MAX);
        let fx: Vec<String> = vec![
            serde_json::json!({"type": "user", "message": {"role": "user", "content": "go"}}).to_string(),
            serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": long}]}}).to_string(),
            serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "and more"}]}}).to_string(),
        ];
        let got = claude(&fx, 1);
        let texts: Vec<usize> = got[0].steps.iter().map(|s| match s { Step::Text { text, .. } => text.chars().count(), _ => 0 }).collect();
        assert!(texts.iter().sum::<usize>() <= TEXT_MAX + 1, "{texts:?}");
        assert!(got[0].truncated);
        assert!(got[0].bytes() >= got[0].reply.len() + TEXT_MAX, "steps count toward the total");
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

    /// The tool steps of a turn, as the wire carries them.
    fn tool_steps(t: &Turn) -> Vec<serde_json::Value> {
        t.steps.iter().map(|s| serde_json::to_value(s).unwrap()).filter(|s| s["kind"] == "tool").collect()
    }

    /// `(name, summary, failed)` of each tool step, for a short assertion.
    fn calls(t: &Turn) -> Vec<(String, Option<String>, bool)> {
        tool_steps(t)
            .iter()
            .map(|s| (s["name"].as_str().unwrap().to_string(), s.get("summary").and_then(|x| x.as_str()).map(str::to_string), s.get("failed") == Some(&serde_json::json!(true))))
            .collect()
    }

    fn call(name: &str, summary: Option<&str>, failed: bool) -> (String, Option<String>, bool) {
        (name.to_string(), summary.map(str::to_string), failed)
    }

    /// A real Claude 2.1.316 session, trimmed (README): each tool step says
    /// what ran, and the two whose result is `is_error: true` (a command
    /// that exited 1, a Read of a missing file) say they failed; a success
    /// writes `false` or nothing and says nothing. No output comes back.
    #[test]
    fn claude_tool_steps_say_what_ran_and_whether_it_failed() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.316-tools.jsonl"));
        let got = claude(&fx, 10);
        let all = without_call_ids(&got);
        for secret in ["SYSTEM-PROMPT-MARKER", "TOOL-OUTPUT-MARKER", "toolu_", "is_error"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        assert_eq!(got.len(), 1, "{got:#?}");
        assert_eq!(
            calls(&got[0]),
            vec![
                call("Bash", Some("ls"), false),
                call("Bash", Some("cat missing-file.txt"), true),
                call("Grep", Some("beta in src"), false),
                call("Glob", Some("**/*.rs"), false),
                call("Read", Some("/private/tmp/claude-501/wtts/repo/nope.txt"), true),
            ]
        );
        assert_eq!(got[0].tools, vec!["Bash", "Bash", "Grep", "Glob", "Read"], "`tools` is as it was, for older clients");
        assert!(tool_steps(&got[0])[0].get("failed").is_none(), "a success carries no flag at all");
    }

    /// `claude_call_image` serves the block of a Read, by its id, and nothing
    /// else: not another tool's result, not an unknown id, not a text result,
    /// and nothing past the cap.
    #[test]
    fn claude_call_image_is_the_read_results_block_by_id() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.319-read-image.jsonl"));
        let got = claude_call_image(&fx, "toolu_01ReadImg").expect("the Read's image");
        assert_eq!((got.mime.as_str(), got.truncated), ("image/png", false));
        assert!(got.b64.starts_with("iVBORw0KGgo") && got.size > 100, "{got:?}");
        assert_eq!(claude_call_image(&fx, "toolu_nope"), None);
        assert_eq!(claude_call_image(&fx, ""), None);
        // the same block under a tool that is not Read
        let other: Vec<String> = fx.iter().map(|l| l.replace("\"name\":\"Read\"", "\"name\":\"mcp__x__shot\"")).collect();
        assert_eq!(claude_call_image(&other, "toolu_01ReadImg"), None, "only a Read's picture is served");
        // a type the viewer does not draw
        let odd: Vec<String> = fx.iter().map(|l| l.replace("\"media_type\":\"image/png\"", "\"media_type\":\"image/tiff\"")).collect();
        assert_eq!(claude_call_image(&odd, "toolu_01ReadImg"), None);
        // past the cap: said, never carried
        let big = "A".repeat(CALL_IMAGE_MAX_B64 + 4);
        let huge: Vec<String> = fx.iter().map(|l| if l.contains("tool_result") { l.replace("iVBORw0KGgo", &big) } else { l.clone() }).collect();
        let cut = claude_call_image(&huge, "toolu_01ReadImg").unwrap();
        assert!(cut.truncated && cut.b64.is_empty());
        // `agent_turns` still carries no pixels
        assert!(!without_call_ids(&claude(&fx, 10)).contains("iVBORw0KGgo"));
    }

    /// A Read of an image (Claude 2.1.319, measured: the fixture's README): the
    /// transcript holds the picture as base64 TWICE in the result entry, and
    /// the turn keeps the call's `file_path` as its summary and no pixels, which
    /// is all the chat's thumbnail needs (it reads the file again through
    /// `read_place_image`, under the place-only rule).
    #[test]
    fn claude_read_of_an_image_is_a_path_and_no_pixels() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/claude-2.1.319-read-image.jsonl"));
        assert!(fx.iter().any(|l| l.contains("\"type\":\"image\"") && l.contains("iVBORw0KGgo")), "the fixture holds the image block");
        let got = claude(&fx, 10);
        let all = without_call_ids(&got);
        for leaked in ["iVBORw0KGgo", "base64", "SYSTEM-PROMPT-MARKER", "originalSize"] {
            assert!(!all.contains(leaked), "{leaked} leaked: {all}");
        }
        assert_eq!(got.len(), 1, "{got:#?}");
        assert_eq!(calls(&got[0]), vec![call("Read", Some("/Users/u/proj/.worktrees-attachments/20261107-4c34b746fcf4773d.png"), false)]);
    }

    /// One Claude assistant entry calling `name` with `input`, then its
    /// result, shaped as 2.1.316 writes them.
    fn claude_call(name: &str, input: serde_json::Value, is_error: Option<bool>) -> Vec<String> {
        use serde_json::json;
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = format!("toolu_{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let mut result = json!({"type": "tool_result", "tool_use_id": id, "content": "TOOL-OUTPUT-MARKER"});
        if let Some(e) = is_error {
            result["is_error"] = json!(e);
        }
        vec![
            json!({"type": "assistant", "timestamp": "2026-11-01T06:40:00.000Z", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}}).to_string(),
            json!({"type": "user", "timestamp": "2026-11-01T06:40:01.000Z", "message": {"role": "user", "content": [result]}, "permissionDecision": {"decision": "allow", "source": "config"}}).to_string(),
        ]
    }

    /// Every Claude tool the chat names gets its line, from the input key
    /// Claude's own tool schema gives it; an MCP tool, whose arguments can be
    /// anything, and an unknown tool get none.
    #[test]
    fn claude_summaries_cover_each_tool_and_no_mcp_tool() {
        use serde_json::json;
        let mut fx = vec![json!({"type": "user", "message": {"role": "user", "content": "go"}}).to_string()];
        for (name, input) in [
            ("Edit", json!({"file_path": "/r/src/a.rs", "old_string": "x", "new_string": "y"})),
            ("MultiEdit", json!({"file_path": "/r/src/b.rs", "edits": []})),
            ("Write", json!({"file_path": "/r/c.md", "content": "TOOL-OUTPUT-MARKER"})),
            ("NotebookEdit", json!({"notebook_path": "/r/n.ipynb", "new_source": "x"})),
            ("WebFetch", json!({"url": "https://example.com/a", "prompt": "summarise"})),
            ("Grep", json!({"pattern": "fn main", "output_mode": "content"})),
            ("Grep", json!({"pattern": "TODO", "path": "crates/worktrees-core"})),
            ("Glob", json!({"pattern": "app/src/**/*.tsx"})),
            ("WebSearch", json!({"query": "tauri 2 page zoom"})),
            ("Agent", json!({"description": "Find the dock's resize code", "prompt": "long", "subagent_type": "Explore"})),
            ("Task", json!({"description": "Survey the fixtures", "prompt": "long"})),
            ("Skill", json!({"skill": "close-out", "args": "now"})),
            ("TodoWrite", json!({"todos": [{"content": "a", "status": "pending"}, {"content": "b", "status": "pending"}, {"content": "c", "status": "completed"}]})),
            ("TodoWrite", json!({"todos": [{"content": "a", "status": "pending"}]})),
            ("mcp__github__get_issue", json!({"owner": "o", "repo": "r", "issue_number": 7})),
            ("mcp__worktrees__report", json!({"to": "(main)", "text": "a message"})),
            ("SomeNewTool", json!({"command": "rm -rf /"})),
        ] {
            fx.extend(claude_call(name, input, Some(false)));
        }
        let got = claude(&fx, 1);
        assert_eq!(
            calls(&got[0]),
            vec![
                call("Edit", Some("/r/src/a.rs"), false),
                call("MultiEdit", Some("/r/src/b.rs"), false),
                call("Write", Some("/r/c.md"), false),
                call("NotebookEdit", Some("/r/n.ipynb"), false),
                call("WebFetch", Some("https://example.com/a"), false),
                call("Grep", Some("fn main"), false),
                call("Grep", Some("TODO in crates/worktrees-core"), false),
                call("Glob", Some("app/src/**/*.tsx"), false),
                call("WebSearch", Some("tauri 2 page zoom"), false),
                call("Agent", Some("Find the dock's resize code"), false),
                call("Task", Some("Survey the fixtures"), false),
                call("Skill", Some("close-out"), false),
                call("TodoWrite", Some("3 items"), false),
                call("TodoWrite", Some("1 item"), false),
                call("mcp__github__get_issue", None, false),
                call("mcp__worktrees__report", None, false),
                call("SomeNewTool", None, false),
            ]
        );
        assert!(!serde_json::to_string(&got).unwrap().contains("TOOL-OUTPUT-MARKER"));
    }

    /// What Claude 2.1.318 recorded for a collapsed paste, and for one queued:
    /// the bubble shows the person's words, and only a WHOLE wrapper counts.
    #[test]
    fn a_collapsed_paste_shows_as_the_words_pasted() {
        use serde_json::json;
        let body = "reply OK only\nl2\n\nl4";
        let whole = format!("\n\n<pasted_content id=\"807e\">\n{body}\n</pasted_content id=\"807e\">\n");
        assert_eq!(pasted_body(&whole), Some(body));
        let user = |c: &str| json!({"type": "user", "message": {"role": "user", "content": c}}).to_string();
        assert_eq!(claude(&[user(&whole)], 1)[0].user.as_deref(), Some(body));
        for keep in [
            format!("look at this: {whole}"),
            format!("{whole} and this"),
            "<pasted_content id=\"a\">\nx\n</pasted_content id=\"b\">".to_string(),
            "<pasted_content id=\"a\">\nx\n</pasted_content id=\"a\">\n<pasted_content id=\"a\">\ny\n</pasted_content id=\"a\">".to_string(),
            "<pasted_content id=\"\">\nx\n</pasted_content id=\"\">".to_string(),
            "<pasted_content>\nx\n</pasted_content>".to_string(),
        ] {
            assert_eq!(pasted_body(&keep), None, "{keep:?}");
            assert_eq!(claude(&[user(&keep)], 1)[0].user.as_deref(), Some(keep.as_str()), "a person's own words stay");
        }
    }

    /// A message queued during a turn keeps its lines, wrapped or not, and
    /// loses every other control and invisible character (measured 2.1.318).
    #[test]
    fn a_queued_message_keeps_its_line_breaks() {
        use serde_json::json;
        for (prompt, want) in [
            ("one\ntwo\u{202E}\u{1b}", "one\ntwo"),
            ("<pasted_content id=\"807e\">\none\ntwo\n</pasted_content id=\"807e\">", "one\ntwo"),
        ] {
            let fx = vec![
                json!({"type":"user","message":{"role":"user","content":"go"}}).to_string(),
                json!({"type":"attachment","attachment":{"type":"queued_command","prompt":prompt,"source_uuid":"s1","delivery_id":"d1","commandMode":"prompt","humanTurn":true}}).to_string(),
                json!({"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","commandUuid":"s1","deliveryId":"d1"}).to_string(),
            ];
            let got = claude(&fx, 1);
            let said: Vec<&str> = got[0].steps.iter().filter_map(|s| if let Step::User { text, .. } = s { Some(text.as_str()) } else { None }).collect();
            assert_eq!(said, [want]);
        }
    }

    /// A summary is transcript text a repository or a prompt can steer, so
    /// every control and invisible format character is REMOVED — the set
    /// `typed_line_ok` refuses: a bidi override, a zero-width space, an
    /// escape — on both harnesses, and a tab reads as a space.
    #[test]
    fn a_summary_carries_no_control_or_invisible_characters() {
        use serde_json::json;
        let mut fx = vec![json!({"type": "user", "message": {"role": "user", "content": "go"}}).to_string()];
        fx.extend(claude_call("Bash", json!({"command": "\n  echo \u{202E}gnp.exe\u{202C}\u{200B} \u{1b}[31mred\tdone\u{2066}  \nsecond line"}), None));
        fx.extend(claude_call("Read", json!({"file_path": "/r/a\u{200D}b\u{FEFF}.rs\u{7}"}), None));
        let got = claude(&fx, 1);
        assert_eq!(calls(&got[0]), vec![call("Bash", Some("echo gnp.exe [31mred done"), false), call("Read", Some("/r/ab.rs"), false)]);

        // Codex: the same characters, as JavaScript escapes in a code-mode
        // script and raw in an argv.
        let cx = vec![
            json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "go"}]}}).to_string(),
            json!({"type": "response_item", "payload": {"type": "custom_tool_call", "name": "exec", "call_id": "c1", "input": "const r = await tools.exec_command({cmd:\"git log \\u202Etxt.\\u202C \\u200Bx\\x1b\"}); text(r.output);\n"}}).to_string(),
            json!({"type": "response_item", "payload": {"type": "function_call", "name": "shell", "call_id": "c2", "arguments": json!({"command": ["bash", "-lc", "ls\u{2067} -la\u{200C}"]}).to_string()}}).to_string(),
        ];
        let got = codex(&cx, 1);
        assert_eq!(calls(&got[0]), vec![call("exec", Some("git log txt. x"), false), call("shell", Some("ls -la"), false)]);
        let all = serde_json::to_string(&got).unwrap();
        assert!(!all.chars().any(|c| c.is_control() || crate::harness::invisible(c)), "{all:?}");
    }

    /// A real Codex 0.160.1 TUI session (launched as worktrees launches it),
    /// trimmed (README): every call is code mode's `exec`, its line is the
    /// script's command or its patch's files, and the command that exited 1
    /// is failed — known only from the `CommandExecution` event between the
    /// call and its output, since the script printed no exit code. The
    /// reply is not read twice from the `AgentMessage` events, and no
    /// output comes back.
    #[test]
    fn codex_tool_steps_say_what_ran_and_whether_it_failed() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/codex-0.160.1-tools.jsonl"));
        let got = codex(&fx, 10);
        let all = without_call_ids(&got);
        for secret in ["BASE-INSTRUCTIONS-MARKER", "TOOL-OUTPUT-MARKER", "REASONING-MARKER", "call_", "exit_code"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        assert_eq!(got.len(), 1, "{got:#?}");
        assert_eq!(
            calls(&got[0]),
            vec![
                call("exec", Some("ls"), false),
                call("exec", Some("cat missing-file.txt"), true),
                call("exec", Some("edit notes.txt; add new.txt; delete old.rs"), false),
            ]
        );
        assert_eq!(got[0].reply, "I’ll run the two commands in order, then make all three file changes in one patch.\n\ndone");
    }

    /// The same Codex through `codex exec` (the same core), trimmed: a
    /// script that throws — a patch that does not apply — is failed by its
    /// `Script failed` head, and the retry that applies is not. A No at an
    /// approval or an Esc writes `aborted by user` (the answers fixture),
    /// which is failed too, as Claude's interrupt is.
    #[test]
    fn codex_a_failed_script_and_an_aborted_call_are_failed() {
        let fx = lines(include_str!("../tests/fixtures/transcripts/codex-0.160.1-exec.jsonl"));
        let got = codex(&fx, 10);
        assert!(!serde_json::to_string(&got).unwrap().contains("TOOL-OUTPUT-MARKER"));
        let patch = Some("edit notes.txt; add new.txt; delete old.rs");
        assert_eq!(
            calls(&got[0]),
            vec![
                call("exec", Some("ls -la"), false),
                call("exec", Some("cat missing-file.txt"), true),
                call("exec", patch, true),
                call("exec", Some("cat notes.txt"), false),
                call("exec", patch, false),
                call("exec", Some("grep -n three notes.txt"), false),
            ]
        );
        // The answers fixture keeps no events (README), so its first call's
        // exit 1 is not known there; its aborted No is.
        let fx = lines(include_str!("../tests/fixtures/transcripts/codex-0.160.1-answers.jsonl"));
        let got = codex(&fx, 10);
        assert_eq!(calls(&got[1]), vec![call("exec", Some("touch c.txt"), true)]);
        assert_eq!(calls(&got[2]), vec![call("exec", Some("touch d.txt"), false)]);
    }

    fn codex_user(text: &str) -> String {
        serde_json::json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}}).to_string()
    }
    fn codex_custom(name: &str, id: &str, input: &str) -> String {
        serde_json::json!({"type": "response_item", "payload": {"type": "custom_tool_call", "name": name, "call_id": id, "input": input}}).to_string()
    }
    fn codex_function(name: &str, id: &str, args: serde_json::Value) -> String {
        serde_json::json!({"type": "response_item", "payload": {"type": "function_call", "name": name, "call_id": id, "arguments": args.to_string()}}).to_string()
    }
    fn codex_output(id: &str, text: &str) -> String {
        serde_json::json!({"type": "response_item", "payload": {"type": "custom_tool_call_output", "call_id": id, "output": [{"type": "input_text", "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"}, {"type": "input_text", "text": text}]}}).to_string()
    }
    /// A `CommandExecution` event as 0.160.1 writes it (`codex-0.160.1-tools.jsonl`).
    fn codex_ran(argv: &[&str], status: &str, exit_code: i64) -> String {
        serde_json::json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {"type": "CommandExecution", "command": argv, "status": status, "exit_code": exit_code, "stdout": "TOOL-OUTPUT-MARKER", "formatted_output": "TOOL-OUTPUT-MARKER"}}}).to_string()
    }

    /// Codex's other shapes: the older function calls carrying a command
    /// (`exec_command`'s `cmd`, `shell`'s argv, `shell_command`'s line) or a
    /// patch; a script calling two tools, a tool other than the two, or a
    /// command it builds (its event's argv fills the line); an MCP tool,
    /// which gets none. And an event is the call's only while that call is
    /// the one awaiting its output.
    #[test]
    fn codex_summaries_cover_each_shape_and_events_go_to_the_one_open_call() {
        use serde_json::json;
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Update File: src/b.rs\n@@\n-x\n+y\n*** Add File: c.rs\n+z\n*** End Patch";
        let fx = vec![
            codex_user("go"),
            codex_function("exec_command", "f1", json!({"cmd": "cargo test -p worktrees-core", "yield_time_ms": 1000})),
            codex_output("f1", "TOOL-OUTPUT-MARKER"),
            codex_function("shell", "f2", json!({"command": ["/bin/bash", "-lc", "make lint\nmake test"], "timeout_ms": 1000})),
            codex_output("f2", "TOOL-OUTPUT-MARKER"),
            codex_function("shell", "f3", json!({"command": ["rg", "-n", "fn main"]})),
            codex_output("f3", "TOOL-OUTPUT-MARKER"),
            codex_function("shell_command", "f4", json!({"command": "git status --short"})),
            codex_output("f4", "TOOL-OUTPUT-MARKER"),
            codex_custom("apply_patch", "f5", patch),
            codex_output("f5", "TOOL-OUTPUT-MARKER"),
            codex_function("apply_patch", "f6", json!({"input": patch})),
            codex_output("f6", "TOOL-OUTPUT-MARKER"),
            codex_custom("exec", "f7", "const a = await tools.exec_command({ \"cmd\": 'pnpm test' });\nconst b = await tools.exec_command({cmd: `pnpm lint`});\ntext(a.output + b.output);\n"),
            codex_output("f7", "TOOL-OUTPUT-MARKER"),
            codex_custom("exec", "f8", "const r = await tools.view_image({path: \"shot.png\"}); text(r);\n"),
            codex_output("f8", "TOOL-OUTPUT-MARKER"),
            codex_custom("exec", "f9", "for (const f of files) { await tools.exec_command({cmd: `wc -l ${f}`}); }\n"),
            codex_ran(&["/bin/zsh", "-lc", "wc -l a.txt"], "completed", 0),
            codex_output("f9", "TOOL-OUTPUT-MARKER"),
            codex_function("mcp__github__get_issue", "f10", json!({"owner": "o", "repo": "r", "issue_number": 7})),
            codex_output("f10", "TOOL-OUTPUT-MARKER"),
            // A patch whose own text calls a tool is still one call.
            codex_custom("exec", "f11", r#"const r = await tools.apply_patch("*** Begin Patch\n*** Add File: run.js\n+await tools.exec_command({cmd: 'make'});\n*** End Patch"); text(r);"#),
            codex_output("f11", "TOOL-OUTPUT-MARKER"),
            // Two calls in flight: their event is nobody's.
            codex_custom("exec", "p1", "const r = await tools.exec_command({cmd:\"false\"}); text(r.output);\n"),
            codex_custom("exec", "p2", "const r = await tools.exec_command({cmd:\"true\"}); text(r.output);\n"),
            codex_ran(&["/bin/zsh", "-lc", "false"], "failed", 1),
            codex_output("p1", "TOOL-OUTPUT-MARKER"),
            // One in flight again: a non-zero exit with no `failed` status counts.
            codex_ran(&["/bin/zsh", "-lc", "true"], "completed", 2),
            codex_output("p2", "TOOL-OUTPUT-MARKER"),
        ];
        let got = codex(&fx, 1);
        let edit = Some("edit src/a.rs, src/b.rs; add c.rs");
        assert_eq!(
            calls(&got[0]),
            vec![
                call("exec_command", Some("cargo test -p worktrees-core"), false),
                call("shell", Some("make lint"), false),
                call("shell", Some("rg -n fn main"), false),
                call("shell_command", Some("git status --short"), false),
                call("apply_patch", edit, false),
                call("apply_patch", edit, false),
                call("exec", Some("pnpm test (+1 more)"), false),
                call("exec", Some("view_image"), false),
                call("exec", Some("wc -l a.txt"), false),
                call("mcp__github__get_issue", None, false),
                call("exec", Some("add run.js"), false),
                call("exec", Some("false"), false),
                call("exec", Some("true"), true),
            ]
        );
        assert!(!serde_json::to_string(&got).unwrap().contains("TOOL-OUTPUT-MARKER"));
    }

    /// The summaries and the flags count toward [`TOTAL_MAX`], and what
    /// goes first is the OLDEST summaries: twenty turns of long commands
    /// keep all twenty turns' words, the newest calls keep their lines, and
    /// one turn of hundreds of calls fits by itself.
    #[test]
    fn many_long_summaries_stay_within_the_budget() {
        use serde_json::json;
        let cmd = |i: usize, j: usize| format!("{i}.{j} {}", "é".repeat(SUMMARY_MAX));
        let mut fx = Vec::new();
        for i in 0..MAX_TURNS {
            fx.push(json!({"type": "user", "message": {"role": "user", "content": format!("turn {i}")}}).to_string());
            for j in 0..12 {
                fx.extend(claude_call("Bash", json!({"command": cmd(i, j)}), Some(j % 2 == 0)));
            }
            fx.push(json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": format!("reply {i}")}]}}).to_string());
        }
        let got = claude(&fx, MAX_TURNS);
        let full: usize = got.iter().map(Turn::bytes).sum();
        assert!(full > TOTAL_MAX, "the fixture must be over the budget to test it: {full}");
        let first = got[0].steps.iter().find(|s| matches!(s, Step::Tool { failed: true, .. })).unwrap();
        let id_len = first.id().unwrap().len();
        let one = first.bytes();
        assert_eq!(one, "Bash".len() + r#","call_id":"""#.len() + id_len + r#","detail":true"#.len() + cmd(0, 0).chars().take(SUMMARY_MAX).collect::<String>().len() + "…".len() + FAILED_BYTES, "a summary, its id and its flag count");

        let (kept, dropped) = within_budget(got);
        assert!(kept.iter().map(Turn::bytes).sum::<usize>() <= TOTAL_MAX);
        assert!(!dropped && kept.len() == MAX_TURNS, "every turn's words stay: summaries go first");
        assert!(kept.iter().all(|t| t.reply.starts_with("reply ")));
        let newest = calls(kept.last().unwrap());
        assert!(newest.iter().all(|(_, s, _)| s.is_some()), "the newest calls keep their lines");
        assert!(calls(&kept[0]).iter().all(|(_, s, _)| s.is_none()), "the oldest went first");
        assert_eq!(calls(&kept[0]).iter().filter(|(_, _, f)| *f).count(), 6, "a call keeps its flag");

        // One turn, alone over the budget.
        let mut fx = vec![json!({"type": "user", "message": {"role": "user", "content": "go"}}).to_string()];
        for j in 0..400 {
            fx.extend(claude_call("Bash", json!({"command": cmd(0, j)}), None));
        }
        let (kept, _) = within_budget(claude(&fx, 1));
        assert!(kept[0].bytes() <= TOTAL_MAX, "{}", kept[0].bytes());
        let c = calls(&kept[0]);
        assert_eq!(c.len(), 400);
        assert!(c.first().unwrap().1.is_none() && c.last().unwrap().1.is_some());

        // Turns whose WORDS are over the budget: the oldest turns go, as many
        // stay as their words allow, and the newest calls keep their lines —
        // a summary never costs a turn, and a turn that went never costs a
        // summary.
        let long = "w".repeat(TEXT_MAX);
        let mut fx = Vec::new();
        for i in 0..MAX_TURNS {
            fx.push(json!({"type": "user", "message": {"role": "user", "content": format!("{i} {long}")}}).to_string());
            for j in 0..3 {
                fx.extend(claude_call("Bash", json!({"command": cmd(i, j)}), None));
            }
            fx.push(json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": long}]}}).to_string());
        }
        let got = claude(&fx, MAX_TURNS);
        let words = |t: &Turn| t.bytes() - t.summary_bytes();
        let fit = got.iter().rev().scan(0, |sum, t| { *sum += words(t); Some(*sum) }).take_while(|&s| s <= TOTAL_MAX).count();
        assert!(fit < MAX_TURNS, "the fixture's words must be over the budget");
        let (kept, dropped) = within_budget(got);
        assert!(dropped && kept.iter().map(Turn::bytes).sum::<usize>() <= TOTAL_MAX);
        assert_eq!(kept.len(), fit, "as many turns as their words allow");
        assert!(calls(kept.last().unwrap()).last().unwrap().1.is_some(), "the newest call keeps its line");
    }

    /// kimi 2.2.0's wire log (`kimi-wire/2.2.0/transcript.jsonl`, real record
    /// shapes, invented content): prompts and text blocks are turns; thinking,
    /// tool output, the system prompt and bookkeeping records are not; an
    /// approval upgrades its call to a permission step (allowed, and session
    /// scope, from the resolution); a question's answer is keyed by the
    /// question's text; an Esc'd turn keeps the part that ran.
    #[test]
    fn kimi_turns_are_prompts_replies_and_what_a_person_decided() {
        let fx = lines(include_str!("../tests/fixtures/kimi-wire/2.2.0/transcript.jsonl"));
        let got = kimi(&fx, 20);
        let all = serde_json::to_string(&got).unwrap();
        for secret in ["THINKING-MARKER", "TOOL-OUTPUT-MARKER", "OLD-MARKER", "NEW-MARKER", "systemPrompt"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        assert_eq!(got.len(), 5, "{got:#?}");

        // Turn 0: text, a Bash approved once, an Edit approved for the
        // session — both permission steps, the Edit's contents left out.
        assert_eq!(got[0].user.as_deref(), Some("Run the probe, apply the patch, then reply DONE."));
        assert_eq!(got[0].at, Some(1760000000));
        assert_eq!(got[0].reply, "Running the probe now.\n\nApplying the patch.\n\nPatched. DONE.");
        assert_eq!(got[0].tools, vec!["Bash".to_string(), "Edit".to_string()]);
        let steps = |t: &Turn| t.steps.clone();
        let s0 = steps(&got[0]);
        assert!(matches!(&s0[1], Step::Permission { tool, allowed: true, scope: None, summary: Some(s), answered_at: Some(_), .. } if tool == "Bash" && s == "Running: ./probe.sh"), "{s0:#?}");
        assert!(matches!(&s0[3], Step::Permission { tool, allowed: true, scope: Some(sc), summary: Some(s), .. } if tool == "Edit" && sc == "session" && s == "Editing: src/parse.rs"), "{s0:#?}");
        // The Edit carried contents: only its path leaves.
        assert!(!all.contains("old_string"), "a path-only tool's contents leaked: {all}");

        // Turn 1: a rejected approval — no call ever lands for it, so the
        // permission step is born of the resolution, where the refusal
        // reached — then a tool nobody was asked about that failed (isError).
        let s1 = steps(&got[1]);
        let kinds1: Vec<&str> = got[1].steps.iter().map(|s| match s {
            Step::Text { .. } => "text",
            Step::Tool { .. } => "tool",
            Step::Permission { .. } => "permission",
            _ => "other",
        }).collect();
        assert_eq!(kinds1, vec!["permission", "text", "tool", "text"], "the rejected approval keeps its place");
        assert!(matches!(&s1[0], Step::Permission { tool, allowed: false, scope: None, answered_at: Some(_), .. } if tool == "Bash"), "{s1:#?}");
        assert!(matches!(&s1[2], Step::Tool { name, failed: true, summary: Some(s), .. } if name == "Read" && s == "missing-manifest.toml"), "{s1:#?}");

        // Turn 2: a question and its answer, picked by the question's text.
        let s2 = steps(&got[2]);
        match &s2[0] {
            Step::Question { questions, answer: Some(a), .. } => {
                assert_eq!(questions.len(), 1);
                assert_eq!(questions[0].header, "Shade");
                assert_eq!(questions[0].options.len(), 3);
                assert_eq!(a.how, "picked");
                assert_eq!(a.answers, vec![QuestionAnswer { question: "Which color?".into(), picked: vec!["Blue".into()], typed: None, note: None }]);
            }
            other => panic!("expected a question, got {other:?}"),
        }

        // Turn 3: a queued worktrees message is an ordinary user turn.
        assert_eq!(got[3].user.as_deref(), Some("[worktrees: message from place \"probe\"] reply QUEUED-OK"));

        // Turn 4: Esc'd — the part that ran stays, with no end marker.
        assert_eq!(got[4].user.as_deref(), Some("Write a long poem, no tools."));
        assert_eq!(got[4].reply, "The river bends");
    }

    /// kimi's call detail: a structured input comes back cleaned; a path-only
    /// tool (Write/Edit) answers its path; a call a person decided at its
    /// approval holds no input, as Claude's model has it.
    #[test]
    fn kimi_tool_call_reads_the_call_it_names() {
        let fx = lines(include_str!("../tests/fixtures/kimi-wire/2.2.0/transcript.jsonl"));
        let mcp = kimi_tool_call(&fx, "tool_status").expect("the MCP call");
        assert_eq!(mcp.tool, "mcp__worktrees__place_status");
        assert_eq!(mcp.input, serde_json::json!({"slug": "probe"}));
        assert!(!mcp.truncated);
        let read = kimi_tool_call(&fx, "tool_read").expect("the failed Read");
        assert_eq!(read.input, serde_json::json!("missing-manifest.toml"));
        // Calls a person decided at their approval are permission steps: their
        // input left with the upgrade.
        assert!(kimi_tool_call(&fx, "tool_probe").is_none());
        assert!(kimi_tool_call(&fx, "tool_edit").is_none());
        assert!(kimi_tool_call(&fx, "tool_gone").is_none());
    }

    /// A wire log read mid-question: the question step is there, un-answered
    /// (the chat does not draw it — the live prompt card does), and a tail
    /// that starts inside a turn starts with `user: null`.
    #[test]
    fn kimi_tail_windows_read_honestly() {
        let fx = lines(include_str!("../tests/fixtures/kimi-wire/2.2.0/transcript.jsonl"));
        let ask = fx.iter().position(|l| l.contains("\"question_ask\"")).unwrap();
        let got = kimi(&fx[..=ask], 20);
        let last = got.last().unwrap();
        assert!(matches!(last.steps.last(), Some(Step::Question { answer: None, answered_at: None, .. })), "{last:#?}");
        let mid = fx.iter().position(|l| l.contains("Patched. DONE.")).unwrap();
        let got = kimi(&fx[..=mid], 20);
        assert_eq!(got.last().unwrap().user, Some("Run the probe, apply the patch, then reply DONE.".into()));
        let from = fx.iter().position(|l| l.contains("Applying the patch.")).unwrap();
        let got = kimi(&fx[from..from + 1], 20);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].user, None, "the window starts inside the turn");
    }

    // ── the turn's id and the whole-turn read (`agent_turn_text`) ──────────

    fn c_user(uuid: &str, text: &str) -> String {
        serde_json::json!({"type":"user","uuid":uuid,"timestamp":"2026-11-06T10:00:00Z","origin":{"kind":"human"},"message":{"role":"user","content":text}}).to_string()
    }
    fn c_say(uuid: &str, text: &str) -> String {
        serde_json::json!({"type":"assistant","uuid":uuid,"timestamp":"2026-11-06T10:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":text}]}}).to_string()
    }
    fn c_tool(uuid: &str, id: &str) -> String {
        serde_json::json!({"type":"assistant","uuid":uuid,"timestamp":"2026-11-06T10:00:02Z","message":{"role":"assistant","content":[{"type":"tool_use","id":id,"name":"Bash","input":{"command":"ls"}}]}}).to_string()
    }
    fn x_line(kind: &str, payload: serde_json::Value) -> String {
        serde_json::json!({"timestamp":"2026-11-06T10:00:00Z","type":kind,"payload":payload}).to_string()
    }
    fn x_started(id: &str) -> String {
        x_line("event_msg", serde_json::json!({"type":"task_started","turn_id":id}))
    }
    fn x_msg(role: &str, kind: &str, text: &str) -> String {
        x_line("response_item", serde_json::json!({"type":"message","role":role,"content":[{"type":kind,"text":text}]}))
    }

    /// A turn's id is its prompt entry's `uuid`, and it is the same on every
    /// read and while the turn grows: a client keeps an expanded bubble open
    /// across the poll by it. A window that starts inside a turn names it by
    /// its first entry, and a repeated uuid names only the first turn.
    #[test]
    fn a_claude_turns_id_is_stable_across_reads_and_growth() {
        let mut fx = vec![c_user("u-1", "first"), c_say("a-1", "one"), c_user("u-2", "second"), c_say("a-2", "two")];
        let ids = |l: &[String]| claude(l, 20).iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&fx), ["u-1", "u-2"]);
        assert_eq!(ids(&fx), ids(&fx), "the same on a second read");
        fx.push(c_tool("a-3", "toolu_1"));
        fx.push(c_say("a-4", "more words"));
        assert_eq!(ids(&fx), ["u-1", "u-2"], "growing the last turn moves no id");
        assert_eq!(claude(&fx, 20)[1].steps.len(), 3);
        // The window starts inside a turn: that turn has the first entry's id.
        let mid = vec![c_say("a-9", "tail of a turn"), c_user("u-3", "next")];
        assert_eq!(ids(&mid), ["a-9", "u-3"]);
        // A uuid that repeats names the first turn only.
        let twice = vec![c_user("u-1", "a"), c_user("u-1", "b")];
        assert_eq!(ids(&twice), ["u-1", ""]);
        // None when the entry has none: absent on the wire, like an older host.
        let bare = serde_json::json!({"type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"x"}}).to_string();
        let t = &claude(&[bare], 20)[0];
        assert!(t.id.is_empty() && serde_json::to_value(t).unwrap().get("id").is_none());
    }

    /// The reply in the owner's photo: a long reply, and a later text step
    /// `agent_turns` drops past its 4000 characters, both come back whole from
    /// the whole-turn read, tool steps between them as `agent_turns` has them.
    #[test]
    fn a_whole_claude_turn_has_the_text_agent_turns_cut_and_dropped() {
        let long = format!("{} END-OF-THE-LONG-BLOCK", "x".repeat(10_000));
        let last = "**One question before step 2**: which branch?";
        let fx = vec![c_user("u-1", "go"), c_say("a-1", &long), c_tool("a-2", "toolu_1"), c_say("a-3", "after the tool"), c_say("a-4", last)];
        let short = &claude(&fx, 5)[0];
        assert!(short.truncated);
        let texts = |t: &Turn| t.steps.iter().filter_map(|s| if let Step::Text { text, .. } = s { Some(text.clone()) } else { None }).collect::<Vec<_>>();
        assert!(!short.reply.contains("which branch"), "the short read loses the question");
        assert!(texts(short).iter().all(|t| !t.contains("which branch")));
        let full = claude_turn(&fx, "u-1").expect("by its id");
        assert!(!full.truncated, "100,000 characters hold it");
        assert_eq!(texts(&full), [long.clone(), "after the tool".to_string(), last.to_string()]);
        let kinds: Vec<&str> = full.steps.iter().map(|s| match s { Step::Text { .. } => "text", Step::Tool { .. } => "tool", _ => "other" }).collect();
        assert_eq!(kinds, ["text", "tool", "text", "text"], "in order, the tool between");
        assert_eq!(full.user.as_deref(), Some("go"));
        assert!(full.reply.is_empty(), "the words are in steps, once");
        assert_eq!(full.id, "u-1");
    }

    /// The whole-turn cap: text past it is cut with an ellipsis, `truncated`,
    /// later text steps go, and a prompt is cut at it too.
    #[test]
    fn the_whole_turn_cap_cuts_and_says_so() {
        let fx = vec![c_user("u-1", &"p".repeat(TURN_TEXT_MAX + 5)), c_say("a-1", &"y".repeat(TURN_TEXT_MAX - 3)), c_say("a-2", "zzzzzzzz"), c_say("a-3", "dropped")];
        let full = claude_turn(&fx, "u-1").unwrap();
        assert!(full.truncated);
        assert_eq!(full.user.as_ref().unwrap().chars().count(), TURN_TEXT_MAX + 1);
        let texts: Vec<usize> = full.steps.iter().filter_map(|s| if let Step::Text { text, .. } = s { Some(text.chars().count()) } else { None }).collect();
        assert_eq!(texts, [TURN_TEXT_MAX - 3, 4], "the second is cut to what is left (3 + the ellipsis); the third goes");
        let exact = claude_turn(&[c_user("u-1", "q"), c_say("a-1", &"y".repeat(TURN_TEXT_MAX))], "u-1").unwrap();
        assert!(!exact.truncated, "a reply exactly at the cap is whole");
    }

    /// Gone: an id the lines no longer hold (the window moved past the turn)
    /// and no id at all answer nothing, and the other turns are untouched.
    #[test]
    fn a_turn_that_left_the_window_is_gone() {
        let fx = vec![c_user("u-2", "kept"), c_say("a-1", "words")];
        assert!(claude_turn(&fx, "u-1").is_none());
        assert!(claude_turn(&fx, "").is_none());
        assert!(claude_turn(&[], "u-2").is_none());
        assert!(codex_turn(&fx, "u-2").is_none(), "a Claude id is not a Codex turn");
        assert!(claude_turn(&fx, "u-2").is_some());
    }

    /// Transcript text is data through the same reader: the system prompt in
    /// an `api-request-blob`, a reminder-only entry and a compact summary's body
    /// never reach the whole-turn read.
    #[test]
    fn the_whole_turn_read_keeps_the_readers_rules() {
        let fx = vec![
            serde_json::json!({"type":"api-request-blob","uuid":"b-1","content":"SYSTEM-PROMPT-MARKER"}).to_string(),
            serde_json::json!({"type":"user","uuid":"r-1","isMeta":true,"message":{"role":"user","content":"<system-reminder>REMINDER-MARKER</system-reminder>"}}).to_string(),
            c_user("u-1", "hello"),
            serde_json::json!({"type":"assistant","uuid":"s-1","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"SIDECHAIN-MARKER"}]}}).to_string(),
            c_say("a-1", "the answer"),
        ];
        let full = claude_turn(&fx, "u-1").unwrap();
        let wire = serde_json::to_string(&full).unwrap();
        for marker in ["SYSTEM-PROMPT-MARKER", "REMINDER-MARKER", "SIDECHAIN-MARKER"] {
            assert!(!wire.contains(marker), "{marker} leaked: {wire}");
        }
        assert!(wire.contains("the answer"));
    }

    /// Codex: `task_started`'s `turn_id` names the turn, the same on every
    /// read and while it grows; a long reply and a late text step come back
    /// whole; a window starting inside a turn takes the id of the first event
    /// that carries it, and none when none does.
    #[test]
    fn a_codex_turns_id_and_whole_text() {
        let long = format!("{} CODEX-END", "w".repeat(9_000));
        let mut fx = vec![
            x_started("t-1"), x_msg("user", "input_text", "go"), x_msg("assistant", "output_text", &long),
            x_started("t-2"), x_msg("user", "input_text", "again"), x_msg("assistant", "output_text", "short"),
        ];
        let ids = |l: &[String]| codex(l, 20).iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&fx), ["t-1", "t-2"]);
        fx.push(x_msg("assistant", "output_text", "a later block of the second turn"));
        assert_eq!(ids(&fx), ["t-1", "t-2"], "growth moves no id");
        let short = codex(&fx, 20);
        assert!(short[0].truncated && !short[0].reply.contains("CODEX-END"));
        let full = codex_turn(&fx, "t-1").unwrap();
        assert!(!full.truncated && matches!(&full.steps[0], Step::Text { text, .. } if text.ends_with("CODEX-END")));
        assert_eq!(codex_turn(&fx, "t-2").unwrap().steps.len(), 2);
        assert!(codex_turn(&fx, "t-9").is_none());
        // Inside a turn: its `turn_context` (or an item event) carries the id.
        let ctx = x_line("turn_context", serde_json::json!({"turn_id":"t-7"}));
        assert_eq!(ids(&[ctx, x_msg("assistant", "output_text", "tail")]), ["t-7"]);
        let item = x_line("event_msg", serde_json::json!({"type":"item_completed","turn_id":"t-8","item":{"type":"AgentMessage"}}));
        assert_eq!(ids(&[item, x_msg("assistant", "output_text", "tail")]), ["t-8"]);
        assert_eq!(ids(&[x_msg("assistant", "output_text", "tail")]), [""], "nothing names it");
        // A turn_context after the typed prompt never gives the NEXT turn this one's id.
        let late = vec![x_started("t-1"), x_msg("user", "input_text", "a"), x_line("turn_context", serde_json::json!({"turn_id":"t-1"})), x_msg("user", "input_text", "b")];
        assert_eq!(ids(&late), ["t-1", ""]);
        // A turn that began with no typed prompt (a wrapper only) leaves its id behind.
        assert_eq!(ids(&[x_started("t-5"), x_started("t-6"), x_msg("user", "input_text", "go")]), ["t-6"]);
    }

    /// The id costs the poll's budget like any other text.
    #[test]
    fn an_id_counts_toward_the_total() {
        let mut t = Turn::new(None, Some("u".into()));
        let bare = t.bytes();
        t.id = "0123456789abcdef".into();
        assert_eq!(t.bytes(), bare + 16);
    }
}
