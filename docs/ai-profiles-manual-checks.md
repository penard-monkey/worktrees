---
title: "AI profiles — manual verification checklist"
---

# AI profiles — manual verification checklist

Everything in this list depends on how a real `claude` binary behaves. None of it
can run in CI: the bats suite drives fake `git`/`tmux` shims and there is no fake
claude, so the automated gates prove that worktrees *composes the right launch*
and nothing about what claude then does with it.

**Re-run this whenever `claude` is upgraded.** The behaviours below were verified
against one version and several are undocumented; a claude release is exactly
what would break them silently.

Record the version you tested:

```sh
claude --version      # verified against: 2.1.220
zsh -lic 'which -a claude'   # >1 install on PATH is its own bug — the app
                             # resolves the LOGIN shell's PATH via fixup_gui_path
```


## Running this without disturbing a worktrees app you already have open

**It does collide, so do not just launch a second build.** Three ways:

| | |
|---|---|
| tmux sessions | Names are `<prefix>-<slug>`, derived from the repo — so a second build computes the **same name** and attaches to the session your open app is using. Closing it in one kills it in the other. |
| App config | Both builds share the bundle id `net.casadelvalle.worktrees`, so both read and write the same `ui-state.json` and `projects.json`. Settings are one debounced blob: last writer wins. |
| No single-instance guard | Two app instances will happily both run, and nothing on screen tells them apart. |

### The sandbox

`app/scripts/sandbox.sh` builds an isolated worktrees for whatever branch you
are on — its own tmux prefix, its own profiles, its own skill store, its own
scratch repo:

```sh
eval "$(app/scripts/sandbox.sh)"            # sets the env + cd's into a scratch repo
```

**How you tell them apart:** every sandbox session is named `sbx-<branch>-<slug>`, while
yours keep their real prefix (`cdv-…`, `worktrees-…`). `tmux ls` shows both, and
they can never collide because the names differ.

```sh
tmux ls | grep sbx-      # sandboxes (one prefix per branch)
tmux ls | grep -v sbx-   # yours, untouched
```

Tear it down with `app/scripts/sandbox.sh --clean`. Note that each
sandbox profile you sign into leaves its own keychain item
(`Claude Code-credentials-<hash>`); the script says so, and cannot remove them,
because worktrees never touches credentials.

Your real `~/.claude` is deliberately **not** isolated — §2 needs to confirm your
global `CLAUDE.md` still loads under a profile, which is the one thing a profile
cannot suppress.

### What needs the app, and what does not

Sections 1, 2, 3, 5, 6, 7, 8 and 9 are all reachable from the sandboxed CLI
(`worktrees new`, `worktrees open`, `worktrees skills`, `worktrees mcp`) with no
app running at all. **Do those first** — they are also the sections covering the
silent failures.

Two sections need the desktop app: §4 (the "restart to apply" badge) and §11
(the headless status report). §11 is app-only by construction — `ai_status_report`
is a Tauri command with no CLI verb behind it, so there is nothing in the
sandboxed CLI to reach it with. Two options:

```sh
app/scripts/sandbox.sh --app
```

That launches the app against the sandbox under a **different bundle identifier
and product name**, so it gets its own config dir and shows as
`worktrees (sbx …)` in the window title.

**Verify the override took**: Settings → Data & Logs should show a path under
`net.casadelvalle.worktrees.sbx`. If it shows the plain identifier instead, the
config merge did not apply and this app is sharing `ui-state.json` with your
installed one — quit the installed app before going further. (The tmux sessions
stay separate regardless, which is the part that would actually have hurt.)

---

## 1. The swap applies at all

- [ ] Create a profile with distinctive rules (e.g. "When asked for the marker,
      reply MARKER-7734").
- [ ] Open a worktree in the app. First launch shows `Not logged in · Run /login`
      — **this is expected**, and the UI should have tagged the profile
      "needs sign-in" beforehand. Sign in once.
- [ ] Ask for the marker. It comes back → `--append-system-prompt-file` applied.
- [ ] `/context` → the rules are in the system prompt, not the message history.
- [ ] `/clear`, ask again → still applied.

## 2. Isolation, and its documented limits

- [ ] `/mcp` → your global MCP servers are **absent** (with inherit-global off).
- [ ] `/context` → your global `~/.claude/skills` are **absent**; only built-in
      skills and the profile's own appear.
- [ ] `/context` → your global `~/.claude/CLAUDE.md` **IS present**. This is the
      known limitation: a profile adds rules, it cannot suppress yours.
- [ ] Relaunch the same place → still signed in (the keychain item is keyed to
      the profile's directory path, which is stable).

## 3. Session adoption and auto-resume — the silent-failure class

These fail *quietly* when wrong, which is why they are on the list.

- [ ] With a session live, `tmux list-panes -a -F '#{pane_current_command}'` →
      the profiled pane reports **`claude`**, not `CLAUDE_CONFIG_DIR=…`.
- [ ] Close the app, reopen → the place still shows as live (adoption works).
- [ ] Have a conversation, close the session, reopen the place → the
      conversation **resumes** (`-r` was passed).
- [ ] While claude is working, the app's busy dot lights for that place.

## 4. Profile changes

- [ ] Edit the profile's rules while a session is live → the topbar shows
      **"restart to apply"**.
- [ ] Restart the session → the badge clears and the new rules apply.
- [ ] Edit an enabled skill's `SKILL.md` while a session is live → the change
      reaches it **without** a restart (skills are symlinked; claude hot-watches
      them). The badge correctly does NOT claim to cover this.

## 5. Fail-closed

- [ ] Break a profile so materialization fails (e.g. `chmod -w` its data dir).
- [ ] Open a place → the pane opens on a **plain shell** with the reason
      printed, and **claude is not running**. This is deliberate: a restrictive
      profile silently not applying is worse than no session.

## 6. Trust is mirrored, never invented

- [ ] Clone a repo you have never opened in claude. Bind a profile. Open it.
- [ ] claude **asks** the trust question. (If it does not, that is a security
      regression — pre-accepting it would make a profile weaker than plain
      claude.)
- [ ] `grep hasClaudeMdExternalIncludesApproved` in the profile's
      `.claude.json` → **absent**, always.

## 7. Credentials never leave the keychain

- [ ] `grep -rl 'sk-ant' "$XDG_DATA_HOME/worktrees/profiles"` → nothing.
- [ ] The profile's `.claude.json` is mode `600` and contains no key/token/
      account-shaped fields.
- [ ] Deleting a profile in the UI **says** that its keychain item and its
      transcripts remain, and where. (worktrees cannot delete the keychain item
      — core has no credential code path by design.)

## 8. The MCP server

- [ ] Enable "worktrees MCP" on a profile, open a place, `/mcp` → `worktrees`
      is connected.
- [ ] Ask it to list places → it reports this repo's worktrees.
- [ ] Without `--mutations` in the stanza, ask it to remove a worktree → it
      refuses and names the missing flag.
- [ ] `worktrees mcp` started outside a git repo exits nonzero.

## 9. Skills

- [ ] Install a skill from a folder → any `allowed-tools`/`hooks`/executable it
      carries is **printed at install**.
- [ ] Install from a git URL carrying capabilities → the UI shows the full
      `SKILL.md` and refuses to install until you confirm.
- [ ] After install, `git log -1` the source repo, then install again → it
      refuses because the branch moved past the reviewed sha.
- [ ] Enable a skill, open a place → it is listed in `/context`.

## 10. The afterglow dot (task-completed state)

Same reason as everything above: the signal is read out of claude's own
`sessions/<pid>.json` probes and `history.jsonl`, both undocumented, so a claude
upgrade can change the shape or the write timing and the dot degrades in silence.

- [ ] Open a place, do NOT prompt → the dot stays empty. (An open-but-quiet
      session probes `status: "idle"`; if a new claude version probes something
      else here, every idle session would light up as finished work.)
- [ ] Prompt something that runs >6s → green blinks, then a **purple static**
      dot when it lands. Hover: "Claude finished just now".
- [ ] Prompt something trivial (<3s) → no ember. The dwell guard needs two
      consecutive 3s ticks.
- [ ] Run claude from a **subdirectory** of a place (`cd app && claude`, prompt
      once) → the ember lands on that PLACE, and `.worktrees.places.json` gains
      no entry named after the subdir.
- [ ] Run claude in the repo ROOT → the ember lands on the `◆ (main)` row, and
      the store gains a `(main)` key.
- [ ] Answer a permission prompt mid-task → amber (waiting) out-ranks the ember;
      the ember returns when the task lands.
- [ ] Quit the app, prompt in a place, relaunch → the ember is there on frame
      one (the 12h `history.jsonl` backfill).
- [ ] In that same window, type `/clear` and nothing else, then relaunch → that
      place does **not** light up, and an already-dim ember does not jump back
      to the freshest tier. (This is the one that catches a change in when
      claude writes transcript `.jsonl` files — the backfill refines a stamp
      using that prompt's OWN session file precisely so housekeeping cannot
      inflate it.)
- [ ] `kill -9` a busy session → it stamps "finished". Known and accepted: the
      busy-exit edge cannot tell completion from death.

How far the ember reaches and in how many brightness steps is Settings →
Navigation → Afterglow; the default is a **12h horizon in 3 steps** (15m ·
1h 44m · 12h, geometrically spaced from a first boundary pinned at 15 minutes).
Change either slider and the checks above still apply — only the boundaries
move. The first step is the one that also lights the project folder.

## 11. Headless status report ("Ask Claude")

The first path in this codebase that launches claude WITHOUT a tmux pane
(`ai_status_report`, lib.rs). Everything a pane gave for free is gone here: you
cannot see what was launched, and you cannot Ctrl-C it — so the guard and the
180s deadline are the only things standing between a broken profile and a
process nobody can find. Neither is reachable from bats: there is no fake
`claude`, and no CLI verb for this at all.

Open the sheet with right-click a worktree → **Status check…**, then the
**Ask Claude** button under the facts.

- [ ] **Profiled repo.** Set a profile with distinctive rules (§1's MARKER-7734
      trick works: "when reporting on a worktree, end with MARKER-7734"). Ask
      Claude → the marker is in the answer, i.e. the run picked up
      `CLAUDE_CONFIG_DIR` and `--append-system-prompt-file`. (`claude --version`
      parity between the profile dir and your global one is the weaker check if
      you would rather not edit rules.)
- [ ] **It is cached, and it survives a restart.** After a successful run,
      `.worktrees.places.json` has a `status_report` key under that place with
      `text` / `epoch` / `verdict` — and NO new `last_worked_epoch` (a report
      *about* a worktree is not work *in* it; stamping it would relight the
      afterglow dot on a cold place). Quit and relaunch the app, reopen the
      sheet → the same text is there without a second spawn.
- [ ] **Unprofiled repo.** Same thing in a repo with no profile: plain env, no
      `CLAUDE_CONFIG_DIR`, still answers.
- [ ] **Fail-closed profile.** Break the profile so `materialize` fails (point
      its rules file at a path that does not exist). Ask Claude → a clean
      *"your AI profile could not be prepared"* error in the sheet, and **no
      spawn at all** (`pgrep -f 'claude -p'` finds nothing while it is running).
      ⚠ This is the check that pays for the guard reading the COMPOSED command
      instead of `match_word`: the fail-closed launch keeps `match_word:
      "claude"` while swapping the command for a `printf … >&2` sentinel, so a
      match_word guard would run printf and cache its error text as claude's
      considered read of the worktree.
- [ ] **Timeout, and no orphan.** Put a shim NAMED `claude` (`#!/bin/sh` /
      `sleep 999`) in a dir prepended to `PATH`, relaunch the app so
      `fixup_gui_path` picks it up. ⚠ It must be named `claude` — an `ai_cmd`
      wrapper under another name trips the guard instead and you will have
      tested the wrong thing. Ask Claude → an error after 180s reading
      *"claude timed out after 180s"*, then `pgrep -f 'sleep 999'` finds
      **nothing**. (That is the `exec ` in the composed `sh -c` line doing its
      job: `run_deadline` kills one pid, so without `exec` the kill lands on
      `sh` and the real process outlives it.)
- [ ] **No planning files.** Run it on a worktree with no task_plan.md /
      findings.md / progress.md / ROADMAP.md → it still answers, from the git
      facts in the JSON alone, and does not claim to have read anything.

---

## 12. Automations (headless run)

The SECOND headless path, and the one that runs over a whole project rather
than one place. `test/automations.bats` covers the contract against a fake
`claude`; what it cannot cover is a real profile, a real transcript, and the
signals a real session leaves behind — which is the whole risk here
(proposal §4.3, open question 4).

Do this in a SCRATCH repo with two or three worktrees, never in a repo you
care about, and with a real AI profile set on it.

```sh
worktrees automations add --name "Sweep" \
  --brief "Look at every worktree in this project and say which ones hold no
           unique work. Propose a lifecycle for each one you are sure about."
worktrees automations run sweep          # 0 clean · 2 findings · 1 failed
worktrees automations show <run-id>
```

- [ ] **It picked up the profile.** Add a distinctive rule to the profile
      (§1's MARKER-7734 trick: "end every report with MARKER-7734") and check
      the marker is in `report.md`. That is `CLAUDE_CONFIG_DIR` +
      `--append-system-prompt-file` reaching a run, which is the whole reason
      the runner goes through `ops::ai_launch_for` rather than spawning
      `claude` itself.
- [ ] **No place's verdict flipped to `active`.** ⚠ THE check this section
      exists for. Run `worktrees status <slug>` for every place BEFORE the run
      and again after; nothing may move from `cold`/`parked`/`at-risk` to
      `active`. `health::assess` folds the newest transcript under a place's
      `~/.claude/projects/<mangled path>` into its activity max, so a run whose
      cwd were a worktree would make every place read `active` the next morning
      — and the sweep would blind the signal it exists to report on. The cwd
      rule is what prevents it; this is how you find out it still holds.
      Cross-check directly: `ls -lt ~/.claude/projects/` — the directory that
      gained a transcript must be `<main root>/.worktrees`'s (mangled), and
      NEITHER the main root's (that is `(main)`'s place) nor any worktree's.
- [ ] **No nav dot lit.** With the app open on that project, no place's
      afterglow dot may appear during or after the run, and no agent dot either.
      Measured 2026-09-22 on claude 2.1.x: `claude -p` writes NO
      `~/.claude/sessions/<pid>.json`, so no agent dot can come from a run.
      Re-measure on a claude upgrade (`ls ~/.claude/sessions | wc -l` before
      and after). If a dot does light, note which place and which file did it:
      the answer is a `skipped`/attribution fix, not a change to the cwd rule.
- [ ] **`last_worked_epoch` is untouched.** `.worktrees.places.json` before and
      after the run must be byte-identical. A report *about* a worktree is not
      work *in* it — the same rule `ai_status_report` follows.
- [ ] **The brief is not on the command line.** While it runs:
      `ps -Ao args | grep 'claude -p'` shows the fixed opener and four paths,
      and NOT the text of your brief. Put a nonsense word in the brief and grep
      for that.
- [ ] **A second run while one is going says so.** `worktrees automations run
      sweep` in another terminal prints *"sweep is already running"* and exits
      0 — and the first run's entry is still the only one in the ledger
      (`worktrees automations runs`).
- [ ] **In-run recursion is closed.** Ask the brief to call `run_automation`
      (add "then start the sweep automation again" to it). It must come back
      saying the tool is not available inside a run, not by spawning one.
      `worktrees automations runs` must show exactly one run.
- [ ] **Applying a proposal does what it says.** `worktrees automations apply
      <run-id> 0 0` on a `set_lifecycle` proposal → `worktrees ls` shows the new
      lifecycle, and `show <run-id>` lists one `actions` entry with `ok: true`.
- [ ] **Cost.** Note the wall-clock seconds and your usage before/after
      (proposal §10.3 asks for a measurement, not a guess). `--max-turns` is 12
      and the deadline is 300s; if a real project's brief regularly hits either,
      that is the evidence for a per-automation knob.

---

## MCP resources in the `@` menu

`make test-mcp` proves the SERVER side end to end (the list, the reads, the
push, and that stdout is never torn). What no suite here can prove is that
claude's picker actually offers them, because there is no fake claude. Re-run
this whenever the `claude` binary is upgraded — the whole feature rests on two
regexes and a cache inside it.

1. In a session in this repo, type `@worktrees` — the places should be listed,
   named by slug, each described as `<lifecycle> · <branch>`. Typing part of a
   slug (`@bug-fix`) should also find it: the client fuzzy-ranks a resource's
   `name` above its uri.
2. Send a message containing one. The reply should show it knew the branch and
   the path WITHOUT calling a tool first — that is the resource being expanded
   and inlined rather than merely named.
3. `(main)` must appear as `place://main` — unless a worktree is literally named
   `main`, which legitimately takes that uri and pushes the main checkout to
   `place://main-2`. A uri ending in a non-word character
   silently resolves to nothing at submit time even though it completes in the
   menu, which is the failure this is most likely to regress into.
4. Create a worktree from the app or the CLI while the session is open, wait a
   few seconds, and type `@` again — the new place should be there with no
   restart. If it is not, `/mcp` → reconnect is the user-level fix, and the
   watcher thread is what regressed.

### The nav drag

Also by hand, for a different reason: the mock records the invoke but has no
tmux to paste into, and driving the real app is off-limits.

0. If your `~/.tmux.conf` sets `base-index 1`, or you have closed and reopened
   panes in the session, do this check anyway — those are exactly the cases the
   pane is located by COMMAND rather than by index for.
1. With a place selected and its session up, drag another place from the nav
   over the terminal. It should show a dashed outline as soon as the drag
   starts (that is the discoverability half) and a solid accent outline once the
   pointer is actually over it.
2. Drop. The token appears in Claude's prompt WITHOUT being submitted, with a
   space either side, and the notice names the place it went to.
3. Press Enter and check the reply knows the branch — that is the mention
   expanding, not just sitting there as text.
4. Drag a place from a DIFFERENT project over the same terminal: the ghost must
   go red with "a session can only reference worktrees from its own project",
   and dropping must do nothing.
5. Drop while Claude is asking a permission question. It must land as text,
   never answer the dialog. **This is the least-verified thing here.** The
   safety argument is that `paste-buffer -p` delivers a PASTE (a TUI handles
   those deliberately) rather than keystrokes; it is NOT that a prompt turns
   bracketed paste off, which it almost certainly does not. If it ever regresses
   the likely cause is a `send-keys` fallback — there must never be one.
6. Drop into a session whose Claude has exited (the tmux session is still up,
   pane 0 is a shell). Nothing should be pasted and the notice should say so,
   rather than the token appearing on a shell prompt under a success message.

**Why this cannot be done headlessly.** `claude -p '… @worktrees:place://x …'`
does NOT expand the mention — and neither does `-p` with a plain `@CHANGELOG.md`,
which is how you can tell it is print mode and not this feature. Mention
expansion lives on the interactive input path. What `-p` CAN prove, and what was
used to check this in: `resources/list` (ask it to list the MCP resources) and
`resources/read` (ask it to read `place://<slug>` with `ReadMcpResourceTool`).
Both exercise the server; only a real typed `@` exercises the picker.

If mentions expand nowhere at all, check the switches that disable the whole
attachment path before suspecting this feature: `CLAUDE_CODE_DISABLE_ATTACHMENTS`,
`CLAUDE_CODE_SIMPLE`, `restricted` mode, and `blockReadsOutsideWorkingDirectories`.

---

## Known-unverifiable

- Whether a future claude version changes the keychain service-name derivation.
  If it does, every profile silently asks to sign in again. There is no way to
  detect this ahead of a release.
- Whether a future claude adds a frontmatter key that grants capability. The
  skill store reports **any** key it does not positively recognise, so a new one
  surfaces as "declares frontmatter `x`" rather than passing silently — but the
  wording will be vague until the key is known.
