---
title: "Cross-project reach — manual checks"
---

# Cross-project reach — manual verification checklist

What the bats suite and the mock harness cannot see about cross-project reach
(`docs/proposals/cross-project.md`). There is no fake Claude, Codex or pi, and
the mock answers instantly, so each item here is done by hand in the real app.
Re-run the section for a harness whenever that harness is upgraded.

## Ground rules

- Use **scratch repos** and a **throwaway tmux server** named with `-L` on
  EVERY tmux call, `kill-server` included. Inside a lane `$TMUX` wins over
  `TMUX_TMPDIR`, so a bare `tmux kill-server` there kills the user's own
  server.
- Register the scratch repos with `worktrees projects add`, and remove them
  afterwards with `worktrees projects rm`.
- Never drive an app someone is using. Use `app/scripts/sandbox.sh --app`,
  and address it by pid only.

```sh
S=~/.cache/worktrees/scratch-xp; mkdir -p $S
for r in alpha beta; do git -C $S init -q $r && git -C $S/$r commit -q --allow-empty -m init; done
worktrees projects add $S/alpha; worktrees projects add $S/beta
alias t='tmux -L xp-check'
```

## 1. Reach, as an agent sees it

- [ ] With `cross_project` off (the default), an agent in `alpha` calling
      `place_status beta:(main)` is refused, and the refusal names the setting.
- [ ] Settings → Agent guidance → Other projects → **Read**, then start a NEW
      session in `alpha`: `list_projects` lists `beta`, with no `root`.
- [ ] A session started BEFORE the change still answers as before (it keeps
      the reach it started with), as the Settings note says.
- [ ] `place_status beta:<a lane with a busy agent>` from `alpha` shows the
      same activity as `beta`'s nav dot at that moment.
- [ ] Mark `beta` private: a new `alpha` session lists it as a name only, and
      `place_status beta:…` is refused with "private".

## 2. Dropping a place into a composer (P1b)

The drop pastes with `paste-buffer -p` (bracketed) and never presses Enter.
A Claude permission prompt is known to ignore a bracketed paste. That is NOT
known for Codex's or pi's composer, so the app refuses a drop into a Codex or
pi pane whose agent is waiting on an approval or a question. These checks
decide whether that refusal can ever be lifted.

For each of **Codex** and **pi**, in a place of `alpha`:

- [ ] Agent idle at its prompt: drag `beta`'s lane onto the pane. The text
      `place beta:<slug>` arrives in the composer, surrounded by spaces, NOT
      submitted.
- [ ] Same, with half a message already typed: the address is inserted at the
      cursor, and the typed text is intact.
- [ ] Agent waiting on an approval (Codex: a command approval; pi: the trust
      modal, `pi_project_trust = "ask"`): the drag chip says the agent is
      waiting, nothing is pasted, and the notice says to drop again later.
- [ ] Record, for the proposal: with the refusal disabled, does a bracketed
      paste during that modal (a) answer it, (b) land in the composer, or
      (c) vanish? Write down the version it was measured on.
- [ ] Ask the agent to act on the address: it calls `place_status` with it.

For **Claude**:

- [ ] Same project: the drop still inserts `@worktrees:place://<slug>`, and
      the mention expands.
- [ ] Another project: the drop inserts the address, not a token.

## 3. Messages across projects (P2)

`report` to `<project>:<slug>` files the message in the RECIPIENT's repo log
(`<its git common dir>/worktrees-messages/`), signed `<this project>:<slug>`.
Allowed at reach `read` and above.

- [ ] A Claude in `alpha` reports to `beta:(main)`; a Claude in `beta`'s
      `(main)` sees it with `messages`, `from` reading `alpha:<slug>`.
- [ ] `beta` answers with `report` to that `from` and `reply_to` its id; the
      `alpha` session wakes from `wait (until: message, slug: beta:(main))`.
- [ ] Same with a **pi** lane in `beta`.

- [ ] Known limits to keep in mind while testing (proposal §4.1): no
      per-sender cap on another project's log (the oldest are pruned first);
      a symlinked log directory is followed; a project renamed mid-session
      signs with its old name until its sessions restart.

### The Codex sandbox (open question Q8 — needs Codex tokens)

Codex's auto-review launch adds exactly one writable root, the lane's own git
common dir (`sandbox_workspace_write.writable_roots`, `codex.rs`
`permission_flags`). That binds Codex's SHELL. Whether it also binds the
`worktrees mcp` server Codex spawns — the process that writes another repo's
log — has never been measured. Until it is, a refused write fails with a
named reason ("could not file the message in beta's log … sandbox …").

- [ ] A **Codex** lane in `beta`, launched in auto-review mode, reports to
      `alpha:(main)`. Record: did the message land in `alpha`'s log, or did
      the tool return the named refusal?
- [ ] From the same lane's shell, `touch "$S/alpha/.git/probe"`. It must be
      REFUSED; if it is not, the sandbox is not in effect and the first
      result proves nothing.
- [ ] Write both results, and the Codex version, into the proposal's §4.1
      and close Q8.

## Afterwards

```sh
t kill-server
worktrees projects rm alpha; worktrees projects rm beta
rm -rf $S
```
