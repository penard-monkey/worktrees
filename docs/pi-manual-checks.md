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
| session JSONL (`message`, `context_edit`, `model_change`) | 0.87.1 captures; 0.99.1 source agrees (`session-manager.js`, session version 3) | `…/fixtures/pi-session/` |
| TUI screens (Working, Retrying, trust modal) | 0.87.1 | `…/fixtures/pi-screen/` |

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
- [ ] While the first turn runs there is **no** session file yet, and
      `place_status` still says `busy` (from the screen's `Working` border).
- [ ] Once pi replies, `<ts>_repo-feat-XXXXXX-g1.jsonl` exists, and the state
      goes `idle` with a `last_done`.
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
- [ ] A resume with no session file (close the lane before its first reply,
      then `open -r`) launches FRESH instead, with `-g2` and a `--model`.

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

## Afterwards

```sh
t kill-server
rm -rf ~/.pi/agent/sessions/--<mangled scratch path>--
```
