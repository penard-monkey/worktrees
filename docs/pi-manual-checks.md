---
title: "pi — manual verification checklist"
---

# pi — manual verification checklist

Everything here depends on how a real `pi` behaves. None of it runs in CI:
the bats suite drives fake `git`/`tmux` shims, and there is no fake pi. The
automated gates prove that worktrees *composes the right launch* and reads the
right file. They prove nothing about what pi then does.

**Re-run this whenever pi is upgraded.** pi moves fast (0.74 → 0.99 in weeks
on one machine). The session format, the list-models table and the TUI screens
are all pinned by fixtures to one version:

| Fixture | Version | Where |
|---|---|---|
| `pi --list-models` table | 0.99.1 | `crates/worktrees-core/tests/fixtures/pi-models/` |
| session JSONL (`message`, `context_edit` + `targetId`, `model_change`) | 0.99.1 live (`pi-session/0.99.1/`), 0.87.1 kept beside it | `…/fixtures/pi-session/` |
| TUI screens (Working, Retrying, trust modal, done, Esc) | 0.99.1 live (`pi-screen/0.99.1/`), 0.87.1 kept beside it | `…/fixtures/pi-screen/` |
| `send` screens (typed inline, submitted, `Steering:` queue) | 0.99.1 live | `…/fixtures/pi-send/0.99.1/` |
| sessions after a hand restart and `/new` (uuid ids, header `cwd`) | 0.99.1 live | `…/fixtures/pi-session/restart/` |

Sections 9–11 (phase 3): run on 2026-09-29, pi 0.99.1,
`lm-studio/qwen/qwen3-coder-480b`, release binary, `TMUX_TMPDIR` scratch
server, throwaway `PI_CODING_AGENT_DIR`. Section 10 passed in full. The
first live runs of it found two `send` bugs that still-frame fixtures could
not: settling on the whole screen, which streams mid-turn, and a `Steering:`
line truncated to an 80-column pane. Sections 9 and 11 are covered by
`test/pi.bats` plus the fixture capture, and were not run by hand.

Last run: 2026-09-29, pi 0.99.1, `lm-studio/qwen3.6-27b`, through the release
binary on a private tmux socket. Sections 1, 2, 4, 5, 6, 7 and 8 passed, and
section 3's model label did; its `/model` switch was not run (only one model
was ready, and the other configured provider is a paid one). What differed from 0.87.1: the
session file now appears at the first USER message, not the first reply.

Record what you tested:

```sh
pi --version                 # built against 0.99.1
worktrees doctor --pi        # the pi and node a PANE gets, and each model's state
```

## Ground rules

- Use a **scratch repo** and a **throwaway tmux server**. `TMUX_TMPDIR=<dir>`
  with `TMUX` unset keeps worktrees off your real tmux server.
- Use a **local model only** (on the machine this was built on,
  `lm-studio/qwen3.6-27b`). No paid provider.
- **Never write under `~/.pi/`.** For dead-host checks, point
  `PI_CODING_AGENT_DIR` at a copy with a fake provider (a `baseUrl` of
  `http://127.0.0.1:1/v1` for a refused port, `http://10.255.255.1:1234/v1`
  for a blackhole).
- pi's session files for the scratch repo land in
  `~/.pi/agent/sessions/--<mangled path>--/`. Delete them afterwards.

```sh
S=~/.cache/worktrees/scratch-pi; mkdir -p $S/tmux
git -C $S init repo && git -C $S/repo commit --allow-empty -m init
printf '.planning/\n' >> $S/repo/.git/info/exclude
cd $S/repo
alias wt='env -u TMUX TMUX_TMPDIR=$S/tmux worktrees'
alias t='env -u TMUX TMUX_TMPDIR=$S/tmux tmux'
```

## 1. Launch

```sh
wt new feat --ai pi --model lm-studio/qwen3.6-27b --no-attach --no-spare --brief "Run ls, then reply DONE."
t ls                                    # repo-feat~agent~pi
t display -p -t '=repo-feat~agent~pi:' '#{pane_start_command}'
```

- [ ] The command carries `--session-dir '<~/.pi/agent/sessions/--…feat-->'`,
      `--no-approve`, `--session-id '<repo-feat-XXXXXX-g1>'`,
      `--model 'lm-studio/qwen3.6-27b'` and the opener, in that order.
- [ ] `pane_current_command` reads `node`, and `worktrees ls --json` /
      `place_status` attribute the session to **pi**, not Claude.
- [ ] `<ts>_repo-feat-XXXXXX-g1.jsonl` appears as soon as the opener is
      submitted (0.99.1 writes at the first USER message; 0.87.1 waited for
      the reply), ending on that user message, and `place_status` says `busy`.
- [ ] When pi replies the state goes `idle` with a `last_done`.
- [ ] `.worktrees.places.json` has `"agent": {"harness":"pi","model":"lm-studio/qwen3.6-27b"}`
      and `"pi_session_gen": 1` for `feat`.

## 2. Resume

```sh
wt close feat -y
wt open feat --ai pi -r --no-attach --no-spare
```

- [ ] The command is the same **minus `--model`**, with the same `-g1` id.
- [ ] pi reopens the conversation on its recorded model (the footer shows it),
      and the JSONL gains no `model_change`.
- [ ] A resume with no session file (close the lane before any message was
      submitted — e.g. while a trust modal holds the opener — then `open -r`)
      launches FRESH instead, with `-g2` and a `--model`.

## 3. Model label

- [ ] `place_status`'s pi agent row carries `"model": "lm-studio/qwen3.6-27b"`.
- [ ] After `/model` inside pi, the row shows the new model at once (pi writes
      `model_change` immediately), before any reply.

## 4. Trust modal (`ask`)

```sh
mkdir -p .pi && echo '{}' > .pi/settings.json && git add .pi && git commit -m pi
WORKTREES_PI_PROJECT_TRUST=ask wt open feat --ai pi --no-attach   # after a close
```

- [ ] Neither `--approve` nor `--no-approve` is on the command line.
- [ ] pi shows `Trust project folder?`, and `place_status` says **waiting**.
- [ ] MCP `send` to the place is refused, naming the prompt. No key reaches
      the pane (the highlighted choice is Trust).
- [ ] Answer it by hand; the opener then runs.

## 5. Allowance

```sh
wt trust pi                        # allow this repo
wt close feat -y && wt open feat --ai pi --no-attach
wt trust pi --revoke
```

- [ ] With the allowance, the command carries `--approve` and pi loads
      `.pi/` without asking.
- [ ] `~/.config/worktrees/config.toml` gained `[trust] pi = ["<repo root>"]`
      and nothing else changed in it. Revoking empties the list.
- [ ] pi's own `~/.pi/agent/trust.json` is untouched (compare its mtime).
      worktrees READS it (section 9) and never writes it.
- [ ] A `.worktrees.toml` with `[trust]` is refused as a hard parse error.

## 6. Dead host

```sh
cp -R ~/.pi/agent/models.json $S/fake/ ...   # or use a prepared fake dir
PI_CODING_AGENT_DIR=$S/fake-refused wt new dead --ai pi --model lm-refused/qwen3.6-27b --no-attach --brief x; echo $?
```

- [ ] Exit **5**, naming the host and "pi was not started"; the worktree and
      `.planning/brief.md` exist; no tmux session was created; no generation
      was recorded.
- [ ] The same with a blackholed `baseUrl` answers within ~3s, not minutes.
- [ ] `--force` launches anyway (pi then retries and fails on its own).
- [ ] MCP `create_worktree` with that model returns the same reason and
      `(exit 5)`.
- [ ] `worktrees doctor --pi` lists the model with `endpoint_unreachable`.

## 7. Esc

- [ ] Esc mid-turn: the JSONL ends on `stopReason: "aborted"`; the dot goes
      idle with no afterglow (an Esc is not finished work).

## 8. Node floor

```sh
PATH=<a dir with node 22.13 first> SHELL=/bin/zsh worktrees doctor --pi
```

- [ ] With pi's own `pi-node` present, a managed pi uses it whatever the
      shell's node is (the launcher puts it first).
- [ ] Without it, a node below pi's `engines.node` is refused with the floor,
      the version and where it came from.

## 9. pi's own trust, and protecting the worktrees server

Use a throwaway `PI_CODING_AGENT_DIR` here: this section writes a
`trust.json`, and the one under `~/.pi` is never written by hand.

```sh
export PI_CODING_AGENT_DIR=$S/agent        # a copy with models.json only
printf '{"%s": true}\n' "$(cd $S && pwd -P)" > $PI_CODING_AGENT_DIR/trust.json
wt close feat -y; wt open feat --ai pi --no-attach
wt doctor --pi
```

- [ ] The command carries `--approve`, and `doctor --pi` says pi's own
      `trust.json` trusts the entry it names.
- [ ] Adding `"<repo root>": false` beside it (the nearest entry wins) gives
      `--no-approve`; `wt trust pi` then gives `--approve` again (the
      allowance grants over pi's distrust).
- [ ] With the allowance on, a place whose `.pi/mcp.json` defines a
      `worktrees` server launches with `--no-approve`, the launch prints the
      "WITHOUT --approve" warning, and pi does not start the repo's server
      (make its command `touch` a marker file).
- [ ] Know the limit: that check runs at LAUNCH. In a lane already running
      with `--approve`, a `.pi/mcp.json` that appears later is picked up by
      pi's `/reload` (ROADMAP).

## 10. The worktrees tools in pi

```sh
wt mcp --install --ai pi     # with the throwaway PI_CODING_AGENT_DIR
cat $PI_CODING_AGENT_DIR/mcp.json
```

- [ ] `mcp.json` holds exactly the `worktrees` entry: `--env
      WORKTREES_MCP_PROVIDER=pi`, `"exposure": "direct"`, `mcp --mutations`.
      `wt mcp --status --ai pi` says `installed, exposure direct` and starts
      no server (`pi mcp list` would).
- [ ] In a pi lane (a new one: running sessions need `/reload`), ask it to
      `report` "hi": `messages` in `(main)` shows it `from` that place.
- [ ] Ask the lane to call `wait` with `until: message, timeout_s: 90` (take
      its unread messages first, or it answers at once): it returns
      `{"event": "timeout", "waited_s": 90}` — not pi's `MCP request timed out
      after 60000ms`.
- [ ] MCP `send` from `(main)` while the lane is idle: `delivered: true`,
      `queued: false`, and the lane answers it.
- [ ] `send` a long task, then a second `send` while it runs: the second says
      `queued: true`, shows as `Steering: …` above pi's prompt, and is
      answered after the first.
- [ ] `send` while the trust modal is up (section 4) is refused and types
      nothing.

## 11. A pi restarted by hand

- [ ] In a lane, `/new` and send a message: the dot stays, and
      `place_status` shows the lane busy then idle.
- [ ] Exit pi (Ctrl-D on an empty prompt) and run a bare `pi` in the pane
      shell: the dot follows the new session.
- [ ] `wt close feat -y && wt open feat --ai pi -r --no-attach`: the command
      carries `--session-id <that session's uuid>` and no `--model`, and pi
      reopens that conversation.

## Afterwards

```sh
t kill-server
rm -rf ~/.pi/agent/sessions/--<mangled scratch path>--
```
