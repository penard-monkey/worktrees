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
