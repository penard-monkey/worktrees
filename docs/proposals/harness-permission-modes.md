---
title: "Codex and Kimi lanes that do not stop at every prompt"
---

# Codex and Kimi lanes that do not stop at every prompt

Status: built, 2026-11-27. The owner's answers: Kimi on Ask When Needed, and
it is the DEFAULT (not Always Ask as proposed); Codex on this host to
Auto-review (his own click in Settings); build the Kimi row; Autonomous sets
Never Ask. Cautious is Always Ask, Balanced (the shipped defaults) Ask When
Needed. `codex_permissions` joined `USER_ONLY_KEYS` beside `kimi_permissions`.

The ask (owner, 2026-11-27): a Codex lane and a Kimi lane each stopped at a
prompt at once. "Is there any way to make them work like claude?" His Claude
lanes run in auto mode: routine work goes ahead, only risky things ask.

Measured on codex-cli 0.172.0, Kimi Code 2.8.1 and pi 1.7.0, in a linked
worktree of a scratch repo (`/tmp/claude-501/wtpm`), on a private tmux server
(`tmux -L`), with each harness's real binary and the owner's real HOME read only.
`~/.codex/config.toml` and `~/.kimi-code/config.toml` had the same md5 before
and after every run.

## Findings

### 1. Codex's "Hooks need review" is a one-time answer, already given

- **The 13 hooks are the owner's own Codex plugins**: 7 from
  planning-with-files (`hooks/codex-hooks.json`) and 6 from vercel
  (`hooks/hooks.json`). Not the repo's (it has no `.codex/`), and not
  worktrees': worktrees passes Codex no hook.
- **Codex keeps the answer in `~/.codex/config.toml`**, one entry per hook:
  `[hooks.state."<plugin>:<file>:<event>:<i>:<j>"]` with `trusted_hash` and
  `trusted_content_hash`. It is global, not per folder or per place.
- **It asks again only when a hook is new or its content changed.** Both
  plugins were re-cached on 2026-11-25 16:59 (`~/.codex/plugins/cache/…`),
  which is the likely cause of today's ask: every hook was "new or changed".
- **The owner already answered it.** The design-simplify-codex rollout begins
  at 12:48:26, the same second `~/.codex/config.toml` was last written, and its
  hooks ran (`hooks.additional_context`). So that was "Trust all and continue".
- **Our flags do not cause it.** A launch with worktrees' exact flags
  (`-c forced_login_method=chatgpt`, the doc fallback `-c`, `--no-alt-screen`,
  `--approve-for-me` + both sandbox `-c`, the trust `-c`) in a brand-new folder
  showed no hooks prompt. Same with no flags.
- **A launch-time answer exists and must not be used.**
  `--dangerously-bypass-hook-trust` and `-c bypass_hook_trust=true` run every
  enabled hook unreviewed. That includes a trusted project's own `.codex/`
  hooks, i.e. commands a cloned repo supplies (ADR 0001). So the only safe
  answer is the user's own "Trust all" in Codex, once per plugin update. To make
  it stick: answer "Trust all and continue" once after a plugin update; it holds
  for every place until a plugin changes again.

### 2. Codex `auto-review` asks almost nothing

- In a TUI lane with worktrees' auto-review flags, a ten-step task ran with
  **no prompt at all**: `git status`, an edit, `git add && git commit`,
  `curl https://example.com`, `rm -rf build` inside the tree, a write to
  `/tmp`, `cat ~/.ssh/config`, a write OUTSIDE the sandbox (Codex retried it
  escalated and its reviewer approved), `rm -rf` of a directory outside the
  tree, and `git commit --amend && git push --force`.
- That is because Codex's reviewer (a second model) weighs **the user's
  authorization**: every step was named in the prompt. A brief counts as that
  authorization. It denies, rather than asks, what it judges risky and
  unauthorized; a deny is not a prompt (the agent is told no and carries on).
- **The two sandbox holes still hold on 0.172.0**: without the extra writable
  root, `git add` in a linked worktree dies on
  `<main>/.git/worktrees/<name>/index.lock` (exit 128, "Operation not
  permitted"); with it, commit works, and network works with
  `network_access=true`.
- **The owner's host is on `ask`** (`host-policy.json`), not the shipped
  default. Balanced, the shipped default, is `auto-review`. So for Codex the fix
  is one click he already has: Settings → Policy → Codex permissions →
  Auto-review.

### 3. Kimi has the modes; worktrees passes none

kimi 2.8.1 has three permission modes. Read from its own policy chain (the
binary is a bundled script; `strings` shows the source) and measured live:

| Mode | Flag | What still asks (measured) |
|---|---|---|
| Always Ask (`manual`, kimi's default) | none | Every shell command that is not read-only (`touch c.txt` asked). Edits INSIDE the place are not asked: kimi auto-approves Write/Edit within a git work tree in every mode. |
| Ask When Needed (`yolo`) | `-y`, `--yolo` | Only its "dangerous" list: `rm -rf` anywhere except `/tmp` (asked for `rm -rf build` inside the tree too), `shutdown`/`reboot`/`halt`, `mkfs`/`wipefs`/`format`, `dd of=/dev/…`, `systemctl poweroff`… and `init 0/6`; questions and plan approval. Also asks for reads or writes of `.git` control paths and sensitive files. |
| Never Ask (`auto`) | `--auto` | Nothing but plan approval in plan mode. `AskUserQuestion` is refused ("decide and continue"). |

In every mode kimi's file tools refuse `.env`, credentials and SSH keys
outright ("Access is blocked to protect secrets"), and `kimi` has **no
sandbox**: in Ask When Needed a shell write outside the place ran unasked.

- **The mode shows on screen**: the status row under the composer starts with
  `Ask When Needed` or `Never Ask`; Always Ask names none. The composer box and
  an approval's rules are drawn as on 2.2.0, so `kimi::read_screen` and
  `read_composer` read all of them (new fixtures `kimi-screen/2.8.1/`, test
  `kimi_2_8_1_screens_read_in_either_permission_mode`). The opener helper's
  empty-composer wait therefore works, and an approval still reads as a modal,
  so the amber dot still lights for one. Not yet driven end to end through
  worktrees (Phase 2's job).
- **The mode is part of the SESSION and survives resume.** kimi records
  `permission.set_mode` in the session's wire log; `kimi -S <id>` with no flag
  came back in Ask When Needed after a `--yolo` launch. The flag wins when given
  (`-S <id> --yolo` on a session left in Always Ask came back in Ask When
  Needed). **There is no flag or variable for Always Ask**: kimi falls back to
  its config's `default_permission_mode`, which we must not write. So a policy
  tightened back to `ask` cannot reach a resumed session; only a fresh session,
  or `/permission` typed in the pane, does.
- **Changing the mode mid-session saves nothing.** `/permission` (a picker:
  Always Ask / Ask When Needed / Never Ask) and `/yolo` change this session
  only; `config.toml` and `tui.toml` kept their md5. Shift+Tab toggles plan
  mode, not permissions. No `/model`-style trap here.

### 4. pi has no gap

pi asks nothing before any tool call, by design (its `docs/security.md`: "it
does not ask for approval before every tool call"). Its `--approve` /
`--no-approve` are project trust (whether repo-local resources load), already
host policy as `pi_project_trust`. Its only narrowing is a tool allow or deny
list (`--tools`, `--exclude-tools`). Nothing to build.

## What "risky" means in each middle mode

| | Claude auto mode | Codex `auto-review` | Kimi Ask When Needed |
|---|---|---|---|
| Who decides | A classifier model, per action | A reviewer model, per escalation, inside a sandbox | A fixed list in kimi's code |
| Runs unasked | Routine reads, edits and commands in the task's scope | Anything inside the workspace-write sandbox (the place, the git common dir, `/tmp`, network: worktrees' holes) | Every tool call not on the list, anywhere on disk |
| Stops it | Destructive or irreversible acts, work outside the task, data leaving the machine (Claude Code's own description; not measured here) | Whatever the reviewer judges risky AND unauthorized; the user's own words raise authorization, so a brief that says "force push" gets one | `rm -rf` (not under `/tmp`), power-off and disk-format commands, `.git` control paths, sensitive files |
| What a stop looks like | Refused, the agent tries another way; asks after repeated refusals | Refused (`deny`), never a prompt | A prompt: the lane waits on you |
| Sandbox | Claude's own, when on | Yes, OS-level | None |

Read in one line: Codex auto-review is closest to Claude's auto mode in
spirit (a model decides, inside a sandbox); Kimi's Ask When Needed is a
denylist with no sandbox, so it is more permissive about what it does
silently, and still stops, as a prompt, for `rm -rf` even inside the place.

## Proposal

One section, **Settings → Policy → Agent permissions**, with one row per
harness that has modes:

- **Codex** (exists today, moved under the heading): Ask / Auto-review / Full
  access. No change in meaning.
- **Kimi** (new): Always Ask / Ask When Needed / Never Ask, passed as nothing /
  `--yolo` / `--auto` on every kimi launch: fresh, resume, `open`, `new`, MCP
  `create_worktree` / `open_place`, the app's main-checkout launch. Never Ask
  gets the same two-click confirm Full access has.
- **pi**: one line saying pi never asks before a tool call, linking its project
  trust row. No control.

Stored and guarded like `codex_permissions`:

- host policy key `kimi_permissions` (`ask` | `ask-when-needed` | `never-ask`)
  in `host-policy.json`, loaded at the engine's boot; below it
  `$WORKTREES_KIMI_PERMISSIONS`, then user config, then the default;
- setter `set_kimi_permissions` is `HostConsole` (a paired device holding every
  capability is refused), and no MCP tool names or takes it;
- `kimi_permissions` in `USER_ONLY_KEYS`, so a repo's `.worktrees.toml` that
  sets it is a parse error (ADR 0001);
- `doctor --kimi` names the mode and where it came from; the launch-anyway and
  `place_status` surfaces say it; `devices.ts`'s "can run commands as you"
  label counts a Kimi mode other than Always Ask as unprompted, as it does
  Codex;
- the presets get a `kimi_access` value: Cautious = Always Ask, Balanced =
  Always Ask (Balanced must stay exactly the shipped defaults), Autonomous =
  Never Ask. A preset bump (`VERSION`) because a preset's meaning changes.

**Default: unchanged, Always Ask.** Nothing loosens without the owner's act.
(Overruled by the owner: the default is Ask When Needed.)

**The resume caveat has to be said where it is set**: "Applies to the next
Kimi launch. A resumed conversation keeps the mode it had; tightening reaches
it only in a new conversation." A launch under Always Ask could also detect a
resumed session last left in another mode (its wire log's newest
`permission.set_mode`) and say so in the launch note. It cannot fix it.

**A per-launch override in the New place dialog: not now.** One host setting
covers the ask; a per-place mode is a second store to keep in sync with
resume's own memory, and the Settings row is one click. Revisit if the owner
wants one risky lane beside careful ones.

**Codex needs no build.** The owner's ask is answered by Auto-review, which
he has. Worth doing: the Agent permissions heading (so the Codex row is found
where Kimi's is), and one hint under it that Codex's "Hooks need review" is
Codex's own, global, asked again after a plugin update, and answered once.

## Questions for the owner

1. Kimi's mode for your lanes: **Always Ask**, **Ask When Needed** or
   **Never Ask**?
2. Codex on this host: switch it from Ask to **Auto-review** now? (yes / no)
3. Build the Kimi row as proposed, default Always Ask? (yes / no)
4. Should Autonomous set Kimi to Never Ask? (yes / no)
