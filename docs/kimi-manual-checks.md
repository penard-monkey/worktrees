---
title: "kimi — manual verification checklist"
---

# kimi — manual verification checklist

Everything here depends on how a real `kimi` (Kimi Code) behaves. None of it
runs in CI: the bats suite drives fake `git`/`tmux` shims and a `kimi` shim
that only logs its argv. The automated gates prove that worktrees *composes
the right launch*, reads the right files and writes the right place files.
They prove nothing about what kimi then does.

**Re-run this whenever kimi is upgraded** (`kimi --version`; it auto-updates
by default, `[upgrade] auto_install` in its `tui.toml`). The wire log, the
screens and the session layout are pinned by fixtures to one version:

| Fixture | Version | Where |
|---|---|---|
| wire log: turns, an approval, a reject, an Esc | 2.2.0 | `crates/worktrees-core/tests/fixtures/kimi-wire/2.2.0/turns.jsonl` |
| wire log: a question modal | 2.2.0 | `…/kimi-wire/2.2.0/question.jsonl` |
| wire log: a session-only `/model` switch (`config.update`) | 2.2.0 | `…/kimi-wire/2.2.0/model-switch.jsonl` |
| wire log: a message queued behind a turn | 2.2.0 | `…/kimi-wire/2.2.0/queued.jsonl` |
| screens: idle, typed, busy-typed, busy-queued, trust, approval, question | 2.2.0 | `…/kimi-screen/2.2.0/` |

The wire fixtures keep only the record types the reader uses, scrubbed of
kimi's system prompt and every reply; the prompts in them are the probe's own.

Last run: 2026-10-19, kimi 2.2.0, `kimi-code/kimi-for-coding-highspeed`,
through the branch's release CLI on a private tmux socket (`TMUX_TMPDIR` with
`TMUX` unset), in a scratch repo. Every box ticked below was seen on that run;
an open box says what covers it instead. Section 10 (the app) is the
end-to-end lane, to be run by hand.

```sh
kimi --version              # built against 2.2.0
worktrees doctor --kimi     # which kimi, trust, MCP, its models, the last opener
```

## Ground rules

- A **scratch repo** and a **throwaway tmux server**:
  `env -u TMUX TMUX_TMPDIR=<dir> worktrees …`, and `tmux -S <dir>/tmux-$UID/default …`
  for every tmux call. Never a bare `tmux kill-server`.
- A **throwaway `XDG_CONFIG_HOME`** for `trust kimi` / `mcp --install --ai kimi`,
  so your real `~/.config/worktrees/config.toml` is not touched.
- **Never write under `~/.kimi-code/`.** kimi writes its own sessions,
  `workspace-trust/` and `workspaces.json` there as you probe; delete the
  scratch repo's sessions by hand afterwards
  (`~/.kimi-code/sessions/wd_<place>_*`) if you want them gone.
- **Never read `~/.kimi-code/credentials` or `oauth`**, and never run
  `kimi provider list --json`: it prints the provider API key. worktrees reads
  only the `[models.*]` tables of `config.toml` (`kimimodels`).
- In kimi's `/model` picker, **Enter makes the choice kimi's DEFAULT** (it
  writes `default_model` into `~/.kimi-code/config.toml`). Use **Alt+S**
  (session-only) when probing.

```sh
S=~/.cache/worktrees/scratch-kimi; mkdir -p $S/tmux $S/xdg
git -C $S init repo && git -C $S/repo commit --allow-empty -m init
cd $S/repo
alias wt='env -u TMUX TMUX_TMPDIR=$S/tmux XDG_CONFIG_HOME=$S/xdg worktrees'
t() { env -u TMUX tmux -S "$(ls -d $S/tmux/tmux-*/default)" "$@"; }
```

## 1. Launch and the deferred opener

```sh
wt new lane1 --ai kimi --model kimi-code/kimi-for-coding-highspeed --no-attach --no-spare \
  --brief "Reply with the single word PONG. Do not run tools."
t display -p -t '=repo-lane1~agent~kimi:' '#{pane_start_command}'
```

- [x] The command is `( '<worktrees>' kimi-opener --pane "$TMUX_PANE" --dir '<place>' --since <ms> … & ); kimi -m '<alias>'; exec …` — **no opener word after kimi** (it has no positional prompt: a trailing word is `unknown command`).
- [x] kimi shows **Trust this folder?** — every place is a new folder to kimi, even under a trusted repo. `place_status` says **waiting**; the helper waits and types nothing.
- [x] Answer it (Enter on *Trust and continue*). The helper then types `Read .planning/brief.md and begin.` into the empty composer, presses Enter, and kimi answers PONG.
- [x] `<git common dir>/worktrees-kimi-opener.log` gains `… lane1: delivered`; `doctor --kimi` and the lane's `place_status` agent row (`opener`) show the same line. The helper process is gone.
- [x] `pane_current_command` reads `kimi`; `place_status` names the provider **kimi**.

## 2. The opener gives up, bounded

Start a NEW tmux server so the bound reaches the pane's environment:

```sh
mkdir -p $S/tmux3
env -u TMUX TMUX_TMPDIR=$S/tmux3 XDG_CONFIG_HOME=$S/xdg WORKTREES_KIMI_OPENER_WAIT_MS=8000 \
  worktrees new lane4 --ai kimi --no-attach --no-spare --brief "Reply PONG4."
sleep 14; tail -1 .git/worktrees-kimi-opener.log
```

- [x] `lane4: not delivered: gave up waiting (the trust prompt was never answered)`.
- [x] The trust modal is still on screen, untouched — nothing was typed into it.

## 3. Activity: busy, waiting, done

**On every upgrade, re-measure the start of a session's log.** A wire log with
events and no turn boundary in its tail reads as a RUNNING turn
(`kimi::wire_turn`). On 2.2.0 `metadata`/`profile.bind` land ~20 ms before the
first `agent.turn.started`, at the first message; a kimi that wrote
`profile.bind` at LAUNCH would show every fresh lane busy forever. Open a lane,
send nothing, and check `place_status` says idle.

- [x] `wait until: idle` called right after `new` (trusted, so no modal) holds while kimi starts and while the opener is pending, then returns `idle` with a `last_done` once the brief's turn ends.
- [ ] An approval (a Bash call) is **waiting** in `place_status` — from kimi's log (`interaction.request`), no screen needed; a question (`AskUserQuestion`) likewise. (The log records were captured live; the reading is unit-tested on them.)
- [x] After the turn: **idle**, `last_done` from `agent.turn.ended`'s own time, and the agent row's `model` is the alias.
- [ ] Esc mid-turn: `agent.turn.ended` with `outcome: aborted` → idle with no `last_done` (an Esc is not finished work). (Captured live; unit-tested.)

## 4. send

```sh
# from an MCP client in the repo (worktrees mcp --mutations):
send {slug: "lane1", text: "Reply with the single word ACK. Do not run tools."}
```

- [x] Idle: `delivered: true, queued: false`, confirmed from a new `turn.prompt` in kimi's log; kimi answers ACK.
- [x] Busy: a second send while kimi works answers `queued: true` — kimi lists it above the composer (`❯ …` / `↑ to edit · ctrl-s to steer immediately`) and logs it only when its turn starts.
- [ ] With an approval open, `send` is refused before any key is typed. (Unit-tested on the captured screen.)
- [ ] While a lane's opener is still pending, `send` is refused ("still being handed its brief").

## 5. Resume

```sh
wt close lane1 -y
wt open lane1 --ai kimi -r --no-attach --no-spare
```

- [x] The command is `kimi -S 'session_<uuid>'` — the place's own session (its `state.json` `cwd` is the place, created no earlier than the place) — and **no `-m`**: kimi reopens on the session's own model.
- [x] With no session for the place, `-r` launches fresh (never `-c`, which picks the newest session for the DIRECTORY, a removed place's included).
- [x] `-S` with an id kimi does not have: `Session "…" not found`, exit 1, nothing created (why the id is resolved, never guessed).

## 6. Model label

- [x] `place_status`'s kimi row carries the alias (`kimi-code/…`).
- [ ] `/model`, then **Alt+S** (session-only): kimi writes `config.update {modelAlias}` at once, before any reply, and the row shows the new alias. `~/.kimi-code/config.toml` is unchanged (compare its mtime). (Measured: the record and the unchanged mtime; the row itself not yet watched.)

## 7. Trust allowance

```sh
wt trust kimi            # this repo
wt new lane2 --ai kimi --no-attach --no-spare
wt trust kimi --revoke
```

- [x] The pane command starts `KIMI_CODE_TRUST_WORKSPACE='1' kimi …`, and kimi shows no trust prompt. kimi writes no `workspace-trust/` record for that launch.
- [x] `$S/xdg/worktrees/config.toml` gained `[trust] kimi = ["<repo root>"]` and nothing else changed. Revoking empties it, and the next launch asks again.
- [ ] A place whose `.mcp.json` defines a `worktrees` server while ours is NOT written launches without the variable (kimi asks).

## 8. The place files (MCP, the rule, the skill)

```sh
wt mcp --install --ai kimi          # kimi_mcp = "mutations" in $S/xdg
wt open lane1 --ai kimi --no-attach --no-spare   # after a close
```

- [x] `<place>/.kimi-code/mcp.json` holds our `worktrees` server (marked with `"//": "written by worktrees for this place…"`), and `git status` in the place is clean (anchored lines in the common `info/exclude`).
- [x] With agent guidance on in a managed repo, `.kimi-code/AGENTS.md` (marked on its first line) and `.kimi-code/skills/worktrees/` are written too. kimi's trust prompt then lists them ("Load project instructions — Check: .kimi-code/AGENTS.md · .kimi-code/skills/"), and the session's `profile.bind` names our AGENTS.md in `agentsMdPaths`.
- [ ] In the lane, kimi lists the `worktrees` MCP tools (`/mcp`), and a `report` from it arrives with `from` = the place.
- [ ] `mcp --uninstall --ai kimi`, close, open: `mcp.json` is gone; a `.kimi-code/` file of the user's beside it is still there.

- [ ] With every place's kimi files gone (settings off, or places removed), the next kimi launch takes worktrees' lines (and their `# worktrees: …` header) out of `.git/info/exclude`. A `.kimi-code/AGENTS.md` of your own written while they are there is named at launch ("…is yours, but worktrees' line … hides it").
- Your own per-place MCP server goes in the repo's `.mcp.json` or kimi's `~/.kimi-code/mcp.json`; `.kimi-code/mcp.json` in a place is worktrees' and rewritten at every launch.

## 9. doctor

- [ ] `sessions_missed` is 0 for every place (`doctor --kimi --json`): kimi's workspace-dir naming still matches `kimi::slug`.
- [x] `worktrees doctor --kimi` lists the aliases from kimi's config with kimi's default marked, the trust posture, the MCP state and the last opener line — and nothing else from that config file.

## 10. The app, end to end (by hand)

Run with `app/scripts/sandbox.sh --app` (isolated identifier and tmux prefix).
Drive it yourself; never script input into it.

- [ ] **New worktree** dialog: *Start with* offers **Kimi**; its model list is kimi's aliases (K3, K2.8…). Create a lane with a brief.
- [ ] The row shows the **K** mark. While kimi asks to trust the folder, the dot is **amber**; answer it in the terminal pane.
- [ ] The opener appears in kimi's composer by itself; the dot goes **green** while it works and leaves the afterglow ring when the turn ends.
- [ ] Trigger an approval (ask it to run `ls`): **amber** until you answer.
- [ ] The wheel scrolls kimi's transcript (kimi draws inline; tmux holds the history).
- [ ] Plan tab → *Generate plan* pastes into kimi's empty prompt; with text typed there it refuses ("unsent text").
- [ ] From another lane's agent, `send` to it lands (or is queued while it works).
- [ ] *Switch agent…* to Claude and back; quit and relaunch the app: the lane resumes its own session (`-S`), on its own model.
- [ ] Remove the place: its `.kimi-code/` files go with it.

## Checklist (docs/adding-a-harness.md), by surface

| Surface | How it is filled for kimi | Seen |
|---|---|---|
| Registry + adapter | `provider::KIMI`, `harness::Kimi` (`-m`, `-S`, no `--name`, trust as env) | bats `test/kimi.bats`, §1, §5 |
| Pane identity | native binary, `pane_current_command` = `kimi`; `node` in its sidecar also reads kimi | unit (`provider.rs`), §1 |
| Resume by place | session whose `state.json` `cwd` is the place and that is no older than it | unit + bats + §5 |
| Model | aliases from `[models.*]` only; never on a resume | unit + bats + §6 |
| `place_status` / `wait` | `kimi::kimi_activity` (wire log; screen only for the trust modal and "is it up yet") | §3 |
| App dot poll | `kimi_tick` + `watch_lane` arm (`every_harness_feeds_the_dot_poll`) | app unit tests; §10 open |
| Re-list on model switch | `LaneTick::models_moved` from the wire log's alias | app unit test; §10 open |
| MCP | per-place `.kimi-code/mcp.json`, `WORKTREES_MCP_PROVIDER=kimi`, mode `kimi_mcp` | bats + §8 |
| `send` | composer by position, receipt from `turn.prompt`, queue from screen | unit + §4 |
| Plan paste / Ask (main) | refuse unless kimi is at an EMPTY composer | app unit test; §10 open |
| Guidance | `.kimi-code/AGENTS.md` + `skills/worktrees/` (kimi never shows MCP `instructions`) | bats + §8 |
| Security | trust asks by default; allowance per repo; protect-ours over it; `kimi_mcp` and `[trust] kimi` are user-only | unit + bats + §7 |
| Frontend | `harness.ts`, row mark, dialog, model picker, mock place `ledger-sync` | harness-check, mock harness; Settings → Kimi is PR2 |
| Wheel | inline, no alt screen, no mouse mode (measured) | §10 open |
| Usage | none yet; the `kimi web` route is PR3 | — |
