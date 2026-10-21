---
title: claude-token-argv — Claude's OAuth token off curl's argv
---

# claude-token-argv: Claude's OAuth token off curl's argv

- **Date:** 2026-10-20
- **Place:** `claude-token-argv` (lane from `(main)`, brief in `.planning/brief.md`)
- **Branch:** `claude-token-argv`
- **PR:** [#520](https://github.com/penard-monkey/worktrees/pull/520), squash-merged as `862b176`
- **Release:** none (the CHANGELOG entry sits under `[Unreleased]`)
- **Planning files:** none (the brief was enough)

## What shipped

- `crates/worktrees-core/src/claude_usage.rs`: the curl for the usage GET is
  built by `usage_cmd(token, url) -> (Command, stdin_bytes)`. The
  `Authorization: Bearer …` header now reaches curl on **stdin** via `-H @-`,
  never argv, where any process can read it with `ps -Ao command`. The
  non-secret headers stay on argv. A token containing CR, LF or NUL is refused
  with `keychain: Claude Code token has an unexpected shape`, which carries no
  token. The TTL, failure floor, 401 re-read, statusline fallback and error
  strings are unchanged.
- `crates/worktrees-core/src/proc.rs`: `run_deadline_stdin(cmd, bytes, secs)`
  shares one body with `run_deadline`. The write happens on a detached thread
  and stdin is closed after it, with the same deadline and kill.
  `run_deadline` keeps `Stdio::null()`.
- ROADMAP: the #511 item is removed. A new item covers `mcpmigrate.rs`'s
  `--env KEY=VALUE` on argv (see Follow-ups).
- CHANGELOG: one `### Fixed` item under `[Unreleased]`.

## Decisions

- **`-H @-` over `--config -`.** Both worked on both CI OSes, measured on
  macOS `/usr/bin/curl` 8.7.1 and on curl 8.5.0 in an `ubuntu:24.04` docker
  image. `-H @-` takes each line verbatim, so there is no quoting layer to get
  wrong. The only bytes it cannot carry (CR, LF, NUL) are refused rather than
  escaped. The reviewer confirmed in curl 8.5.0's source (`file2memory` +
  `strtok(…, "\r\n")`) that this is exactly the right set, and measured that a
  NUL drops every header after it.
- **The writer thread is never joined.** A child that never reads stdin would
  otherwise block the caller on a full pipe. Once the child is killed the
  writer gets EPIPE (Rust ignores SIGPIPE) and ends. The proc doc says a
  grandchild holding stdin keeps it blocked longer.
- **Curl stays.** There is no HTTP crate and no second TLS stack, and kimi's
  raw `TcpStream` works only because its endpoint is plain loopback HTTP.

## Dead ends / gotchas

- **The audit grep missed a secret on argv.** I swept every spawn for
  `token|secret|api_key|bearer|authorization|password` on an `.arg`/`.env`
  line and declared everything else clean. Review found `mcpmigrate.rs:180`
  building `--env {key}={value}` from a user's literal MCP env values (usually
  API keys). A `format!` of a generic key/value pair contains none of those
  words. The PR body was corrected and the gap is now in ROADMAP. The lesson:
  an argv audit has to follow where VALUES come from, not grep for names.
- **The byte-exact loopback test passes on the old code**, because argv is
  byte-exact too. It guards the stdin ENCODING, not the fix, so it was shown
  RED against a naively quoted `--config -` (the header arrived cut off at the
  `"`). The argv and line-break tests are the ones that fail on the old
  builder.
- **Mutating the stdin write to synchronous** made the deadline test fail after
  the child's full 30s rather than hang forever, because `sleep` exiting closes
  the pipe. RED either way, and slow.

## Verification

- Every new test was shown RED first: the argv test, the line-break refusal,
  the byte-exact test against the wrong encoding, and the never-reads deadline
  test against a synchronous write. Each mutation was restored by editing back
  and then `touch`ed.
- Local gates: release build, `make test` (534, 0 `not ok`), `make lint`,
  `cargo test -p worktrees-core` (864 passed), `-p worktrees-cli`,
  `-p app --lib` (165), `tsc --noEmit`, `cargo check -p app`.
- CI: run 39501082790 on head `0bdca29`, all 9 jobs green.
- Synthetic tokens only. The keychain was never read and the real token was
  never touched.
- Not run, and David's to do: the live meter. Open the app with Claude signed
  in and confirm the bars are `oauth`, not dimmed. During a refresh, run
  `ps -Ao command | grep "[c]url -s --max-time 10"`; it should show `-H @-`
  and no `Bearer`.

## Follow-ups

- ROADMAP: **"Copying an MCP server to Codex puts its literal env values on
  argv."** codex's CLI has no stdin form for `--env`, and worktrees never
  writes `~/.codex`, so it needs a decision first.
- Review nit 4, left alone deliberately: an empty `accessToken` still sends a
  bare `Bearer ` (401, then retry, then the http-401 error), and a
  `~/.curlrc` with `-v` writes the header to stderr, which is captured and
  never logged. Both are existing behaviour, and #520 makes neither worse.
