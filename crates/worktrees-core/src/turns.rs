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
//! that is a turn. The one exception is Claude's `attachment` whose
//! `queued_command` was absorbed mid-turn: it is a person's message inside
//! that turn, not a new `user` entry. Its `rendered` reminder is never read.
//! Thinking and reasoning are left out as well.
//!
//! Text is DATA. The reply is what the agent wrote, and what the user typed
//! may itself be a message another place sent; the row says so on its answer.
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
    /// The turn in order: `reply`'s blocks, `tools`, and what was asked and
    /// answered between them, each step with its own time.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
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
        #[serde(skip)]
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
        Turn { at, user, reply: String::new(), tools: Vec::new(), truncated: false, steps: Vec::new() }
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
            _ => Step::Tool { at, name: name.to_string(), id, summary: summary_of(name, input), failed: false },
        });
    }

    fn bytes(&self) -> usize {
        self.user.as_ref().map_or(0, String::len)
            + self.reply.len()
            + self.tools.iter().map(String::len).sum::<usize>()
            + self.steps.iter().map(Step::bytes).sum::<usize>()
    }

    /// The part of [`Turn::bytes`] that is its tool calls' summaries.
    fn summary_bytes(&self) -> usize {
        self.steps.iter().map(|s| if let Step::Tool { summary: Some(s), .. } = s { s.len() } else { 0 }).sum()
    }
}

/// The last `n` turns (at most [`MAX_TURNS`]) of a Claude transcript's `lines`,
/// oldest first. A `user` entry whose content is text starts a turn, unless it is
/// claude's own bookkeeping (`isMeta`, a `<command-…>` / `<local-command-…>`
/// wrapper, an interruption marker); a tool result does not. A human
/// `queued_command` attachment belongs inside the current turn only when its
/// queue removal says `absorbed_mid_turn`. Other 2.1.316 consumption reasons
/// are `delivered_to_agent`, `delivered_as_tool_result` and
/// `carried_to_background`; none is a mid-turn human message for this row.
pub fn claude(lines: &[String], n: usize) -> Vec<Turn> {
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
        if !(l.contains("\"type\":\"user\"") || l.contains("\"type\":\"assistant\"") || l.contains("\"type\":\"attachment\"") || l.contains("\"type\":\"queue-operation\"")) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        if v.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) || v.get("isMeta").and_then(|s| s.as_bool()) == Some(true) {
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
                let text: String = raw.chars().filter(|c| !c.is_control() && !crate::harness::invisible(*c)).collect();
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
                if let Some(text) = text.filter(|t| is_prompt(t)) {
                    turns.push(Turn::new(at, Some(text)));
                } else if let (Some(serde_json::Value::Array(blocks)), Some(turn)) = (content, turns.last_mut()) {
                    answered(turn, &v, blocks, at);
                }
            }
            Some("assistant") => {
                let Some(serde_json::Value::Array(blocks)) = content else { continue };
                let turn = current(&mut turns, at);
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

/// A command's argv as the line a person typed: `["/bin/zsh", "-lc",
/// "<cmd>"]` (what Codex 0.160.1 runs) is `<cmd>`; anything else, its words.
fn argv_summary(argv: &[serde_json::Value]) -> Option<String> {
    let words: Vec<&str> = argv.iter().filter_map(|w| w.as_str()).collect();
    match words.as_slice() {
        [sh, "-lc" | "-c", cmd] if matches!(sh.rsplit('/').next(), Some("bash" | "zsh" | "sh")) => one_line(cmd),
        _ => one_line(&words.join(" ")),
    }
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

/// The files a patch touches, from its `*** Update File:` / `*** Add File:`
/// / `*** Delete File:` headers: `edit a.rs, b.rs; add c.rs`.
fn patch_summary(patch: &str) -> Option<String> {
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
    one_line(&groups.iter().map(|(v, paths)| format!("{v} {}", paths.join(", "))).collect::<Vec<_>>().join("; "))
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
    let mut turns: Vec<Turn> = Vec::new();
    // The calls awaiting their output, by `call_id`, in the last turn.
    let mut open: Vec<String> = Vec::new();
    for l in lines {
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
                if let Some(Step::Tool { failed, summary, .. }) = turns.last_mut().and_then(|t| t.steps.iter_mut().find(|s| s.id() == Some(id.as_str()))) {
                    *failed |= item.get("status").and_then(|s| s.as_str()) == Some("failed") || item.get("exit_code").and_then(|c| c.as_i64()).is_some_and(|c| c != 0);
                    if summary.is_none() {
                        *summary = item.get("command").and_then(|c| c.as_array()).and_then(|argv| argv_summary(argv));
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
                    turns.push(Turn::new(at, Some(typed.join("\n"))));
                    open.clear();
                }
            }
            (Some("message"), Some("assistant")) => {
                let turn = current(&mut turns, at);
                for t in texts("output_text") {
                    turn.say(at, &t);
                }
            }
            (Some("function_call" | "custom_tool_call"), _) => {
                if let Some(name) = p.get("name").and_then(|n| n.as_str()) {
                    let t = current(&mut turns, at);
                    t.tools.push(name.to_string());
                    let id = p.get("call_id").and_then(|c| c.as_str()).unwrap_or("").to_string();
                    if !id.is_empty() {
                        open.push(id.clone());
                    }
                    t.steps.push(if name == "request_user_input" {
                        let args = p.get("arguments").and_then(|a| a.as_str()).and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok());
                        Step::Question { at, answered_at: None, questions: questions_of(args.as_ref()), answer: None, id }
                    } else {
                        Step::Tool { at, name: name.to_string(), id, summary: codex_summary(name, p), failed: false }
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
    finish(turns, n)
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
            t.truncated |= clip_steps(&mut t.steps);
            t
        })
        .collect()
}

/// The steps held to the reply's caps: TEXT_MAX characters of reply text
/// across all `text` steps (any past it go), and TEXT_MAX for each other
/// string. True when anything was cut.
fn clip_steps(steps: &mut Vec<Step>) -> bool {
    let mut cut = false;
    let mut left = TEXT_MAX;
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
        let (out, was) = clip(s);
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
            Step::Tool { name, summary, failed, .. } => name.len() + opt(summary) + if *failed { FAILED_BYTES } else { 0 },
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

/// `turns` cut to [`TOTAL_MAX`] bytes of text, newest kept. Whole turns go
/// first, oldest first, counted WITHOUT their tool calls' summaries, so a
/// summary never costs a turn; then the kept turns' summaries, oldest first,
/// until the rest fits — each call keeping its name and its flag. The flag
/// says whether any turn went. The newest turn always stays.
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

    fn ts(s: &str) -> i64 {
        crate::sysclock::parse_iso8601(s).unwrap()
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
        let all = serde_json::to_string(&got).unwrap();
        for secret in ["SYSTEM-PROMPT-MARKER", "THINKING-MARKER", "noOutputExpected", "drwxr-xr-x", "permissionDecision", "toolu_"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        assert_eq!(got.len(), 7, "{got:#?}");
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
        assert_eq!(b[2], json!({"kind": "tool", "at": ts("2026-10-31T18:28:15.685Z"), "name": "Bash", "summary": "ls -la"}));
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
                {"kind": "tool", "at": ts("2026-10-23T18:01:02.000Z"), "name": "exec_command", "summary": "make test"},
                {"kind": "tool", "at": ts("2026-10-23T18:01:10.000Z"), "name": "apply_patch"},
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
        let all = serde_json::to_string(&got).unwrap();
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
        let all = serde_json::to_string(&got).unwrap();
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
        let one = got[0].steps.iter().find(|s| matches!(s, Step::Tool { failed: true, .. })).unwrap().bytes();
        assert_eq!(one, "Bash".len() + cmd(0, 0).chars().take(SUMMARY_MAX).collect::<String>().len() + "…".len() + FAILED_BYTES, "a summary and its flag count");

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
}
