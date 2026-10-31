Hand-written transcripts for `worktrees_core::turns`. The SHAPES are real —
the entry types and fields a Claude Code 2.1.309 session wrote on
2026-10-23 (a scratch session, surveyed type by type) and a codex-tui 0.160.1
rollout wrote on this machine (its record and content-block types only) —
and the content is invented. Every place a turn must NOT be
read from (Claude's `api-request-blob` system prompt and CLAUDE.md, an
attachment, a sidechain, thinking, a tool's output, claude's own
`<command-…>` / `<local-command-…>` wrappers; Codex's `base_instructions`,
`developer` messages, the `<environment_context>` and AGENTS.md blocks it
injects, reasoning, tool output) carries a `…-MARKER` string, and the tests
fail if any of them comes back.

`claude-2.1.316-answers.jsonl` is different: it is a REAL transcript, trimmed.
A scratch Claude Code 2.1.316 session (Haiku, a scratch repo, 2026-10-31)
was asked, and answered at its own prompts:
- a question picked from its options (Banana);
- a question answered in "Type something." (a sleepy otter);
- a multi-select (Cheese, Mushrooms);
- a question answered with "Chat about this";
- a Bash permission answered Yes;
- one answered No with a note, then one answered Yes with a note;
- a plan sent back with feedback, then approved;
- a Write answered No.

Only its `user` and `assistant` entries are kept, with the fields the
reader looks at (plus the ids and version). Thinking is replaced by
`THINKING-MARKER`, the home directory and user name are replaced by `u`, and
one `api-request-blob` line holding `SYSTEM-PROMPT-MARKER` is added back.
Everything else is as Claude wrote it.

`codex-0.160.1-answers.jsonl` is a REAL rollout, trimmed the same way. A
scratch codex-cli 0.160.1 session (`-a on-request -s read-only`, a scratch
repo, 2026-10-31) asked to run `touch b.txt` (answered Yes) and `touch
c.txt` (answered "No, and tell Codex what to do differently", then "use
d.txt instead" typed into the composer), then in Plan mode asked one
`request_user_input` question (answered Green, with the note "a darker
green please"). Kept: user and assistant messages, function and custom tool
calls and their outputs, and the task events. A `session_meta` holding
`BASE-INSTRUCTIONS-MARKER` is added back. Neither approval's answer is in it:
Codex writes the escalated call and its output, not the decision.
