---
title: "Claude paste rules — manual verification checklist"
---

# Claude paste rules — manual verification checklist

`agent_send` types into Claude by pasting, and everything it relies on about how
a paste behaves is undocumented Claude behaviour. No suite can notice it changing:
the fixtures are captures of one version, and there is no fake Claude.

**Re-run this whenever `claude` is upgraded.** The measured values, and the code
each one reaches, are in
`crates/worktrees-core/tests/fixtures/claude-screen/2.1.318/multiline/README.md`.

If a rule moved, a send fails safe rather than wrong: it stops with words and
presses no Enter (`prompt::feed_claude`, `prompt::composer_holds`). So the symptom
of a moved rule is a chat that keeps refusing ordinary multi-line messages, or
messages that arrive wrapped in `<pasted_content>`, not lost or mixed text.

## Set up

Use a scratch repo and a private tmux server, never your own. Launch as worktrees
does, with `DISABLE_AUTOUPDATER=1`. Isolate worktrees with `XDG_CONFIG_HOME`, and
never give a scratch Claude its own `XDG_DATA_HOME` (AGENTS.md):

```sh
tmux -L wte -f /dev/null start-server   # or let `worktrees new` start it
export TMUX=/private/tmp/tmux-$UID/wte,0,0 WT_E2E_TMUX=/private/tmp/tmux-$UID/wte
export XDG_CONFIG_HOME=<scratch>/cfg DISABLE_AUTOUPDATER=1
cd <scratch repo> && worktrees new lane --ai claude --model haiku --no-attach
```

Then send through the real path, one step per run:

```sh
WT_E2E_REPO=<scratch repo> WT_E2E_SLUG=lane WT_E2E_SESSION=<session> \
  WT_E2E_STEP=send WT_E2E_TEXT=$'one\ntwo' \
  cargo test -p worktrees-api --test agents_e2e -- --ignored --nocapture
```

Clear the box with Esc, twice. **Do not use Ctrl-C in a script:** a second one
inside its window quits Claude, and every later paste lands at a shell prompt.

## The checks

Read the record, not the screen: `user` entries of the session's transcript under
`~/.claude/projects/`.

1. Paste `a\nb\nc` and then `a\nb\nc\nd` once each, and 800, 850 and 900 characters
   once each. Note where the placeholder starts. `CHUNK_BYTES` must stay below it.
2. Send 3 lines, then 16 KB of real code. Each record must equal the text, with no
   `<pasted_content>`.
3. Send a message whose later lines are `/clear`, `!ls`, `#x`. They must stay text
   and no command may run. Then a later line `@<real file>`: note whether it attaches.
4. Send a message ending in `;`. The record must keep it.
5. Resize to 40, 24, 16, 12, 11 and 10 rows and send 60 short lines by chunks, with no
   Enter. Count the box's rows at each height against `Pace::of`, and note where
   a chunk with a line break becomes a placeholder.
6. Send 15 KB in small pastes and watch for the fold near 10,000 characters. Then
   submit: the record must be whole.
7. Send 1500 short lines with nobody typing: delivered. Then send it again and, about
   half a second in, `tmux send-keys -l ' TYPED'`. It must answer `stopped: true`,
   and no new record may hold `TYPED`. Repeat at a few moments.
8. In 10 rows, send 4 lines. It collapses and must still deliver, and `turns` must show
   the words, not the tags. Note how long `paste again to expand` lasts after Enter,
   and that Stop says to try again rather than "not working" meanwhile.
9. Send during a running turn. The queued message must keep its lines in `turns`.

Add a fixture directory for the new version, re-pin the table, and update the
constants if a value moved.
