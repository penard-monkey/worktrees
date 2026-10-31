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
//!
//! **A turn's `steps` are the same turn in order**, each with its own time:
//! the reply's blocks, the tools it called, and what a person was ASKED and
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
    /// A tool it called that nobody was asked about.
    Tool {
        at: Option<i64>,
        name: String,
        #[serde(skip)]
        id: String,
        #[serde(skip)]
        summary: Option<String>,
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
            _ => Step::Tool { at, name: name.to_string(), id, summary: summary_of(name, input) },
        });
    }

    fn bytes(&self) -> usize {
        self.user.as_ref().map_or(0, String::len)
            + self.reply.len()
            + self.tools.iter().map(String::len).sum::<usize>()
            + self.steps.iter().map(Step::bytes).sum::<usize>()
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
                    result.and_then(|r| r.get("answers")).and_then(|a| a.as_object()).map(|given| Answer {
                        how: "picked",
                        answers: questions
                            .iter()
                            .filter_map(|q| given.get(&q.question).and_then(|a| a.as_str()).map(|a| picked(q, a)))
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
            _ => {}
        }
    }
}

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

/// What a tool call would do, in a line: a command's first line, the file
/// it writes or reads, the address it fetches. Nothing for any other tool.
fn summary_of(name: &str, input: Option<&serde_json::Value>) -> Option<String> {
    let key = match name {
        "Bash" => "command",
        "Edit" | "MultiEdit" | "Write" | "Read" => "file_path",
        "NotebookEdit" => "notebook_path",
        "WebFetch" => "url",
        _ => return None,
    };
    let line = input?.get(key)?.as_str()?.lines().next()?.trim();
    (!line.is_empty()).then(|| clip_to(line, SUMMARY_MAX).0)
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
                    turn.say(at, &t);
                }
            }
            (Some("function_call" | "custom_tool_call"), _) => {
                if let Some(name) = p.get("name").and_then(|n| n.as_str()) {
                    let t = current(&mut turns, at);
                    t.tools.push(name.to_string());
                    let id = p.get("call_id").and_then(|c| c.as_str()).unwrap_or("").to_string();
                    t.steps.push(if name == "request_user_input" {
                        let args = p.get("arguments").and_then(|a| a.as_str()).and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok());
                        Step::Question { at, answered_at: None, questions: questions_of(args.as_ref()), answer: None, id }
                    } else {
                        Step::Tool { at, name: name.to_string(), id: String::new(), summary: None }
                    });
                }
            }
            (Some("function_call_output"), _) => {
                let id = p.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                let Some(Step::Question { answered_at, questions, answer, .. }) = turns.last_mut().and_then(|t| t.steps.iter_mut().find(|s| s.id() == Some(id))) else { continue };
                let out = p.get("output").and_then(|o| o.as_str()).and_then(|o| serde_json::from_str::<serde_json::Value>(o).ok());
                *answered_at = at;
                *answer = out.as_ref().and_then(|o| o.get("answers")).and_then(|a| a.as_object()).map(|given| Answer {
                    how: "picked",
                    answers: questions.iter().filter_map(|q| given.get(&q.key).map(|a| codex_answer(q, a))).collect(),
                });
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
            Step::Text { text, .. } => text.len(),
            Step::Tool { name, .. } => name.len(),
            Step::Question { questions, answer, .. } => {
                questions.iter().map(|q| q.header.len() + q.question.len() + q.options.iter().map(|o| o.label.len() + o.description.len()).sum::<usize>()).sum::<usize>()
                    + answer.iter().flat_map(|a| &a.answers).map(|a| a.question.len() + a.picked.iter().map(String::len).sum::<usize>() + opt(&a.typed) + opt(&a.note)).sum::<usize>()
            }
            Step::Permission { tool, summary, scope, note, .. } => tool.len() + opt(summary) + opt(scope) + opt(note),
            Step::Plan { plan, plan_file, note, .. } => plan.len() + opt(plan_file) + opt(note),
        }
    }
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

    fn ts(s: &str) -> i64 {
        crate::sysclock::parse_iso8601(s).unwrap()
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
        assert_eq!(b[2], json!({"kind": "tool", "at": ts("2026-10-31T18:28:15.685Z"), "name": "Bash"}));
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
                {"kind": "tool", "at": ts("2026-10-23T18:01:02.000Z"), "name": "exec_command"},
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
}
