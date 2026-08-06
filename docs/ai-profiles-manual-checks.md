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

`app/scripts/profiles-sandbox.sh` builds an isolated worktrees — its own tmux
prefix, its own profiles, its own skill store, its own scratch repo:

```sh
cargo build --release -p worktrees-cli
eval "$(app/scripts/profiles-sandbox.sh)"   # sets the env + cd's into the scratch repo
```

**How you tell them apart:** every sandbox session is named `wtsbx-<slug>`, while
yours keep their real prefix (`cdv-…`, `worktrees-…`). `tmux ls` shows both, and
they can never collide because the names differ.

```sh
tmux ls | grep wtsbx     # the sandbox
tmux ls | grep -v wtsbx  # yours, untouched
```

Tear it down with `app/scripts/profiles-sandbox.sh --clean`. Note that each
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

Only §4 (the "restart to apply" badge) needs the desktop app. Two options:

- **Quit your installed app first**, then `cd app && pnpm tauri dev` from the
  sandboxed shell. It inherits the sandbox's profiles and tmux prefix. Cleanest.
- **Run both**, accepting that `ui-state.json` and `projects.json` are shared —
  so a settings or project-list change in one can be overwritten by the other.
  The tmux sessions still stay separate, which is the part that would have hurt.

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

---

## Known-unverifiable

- Whether a future claude version changes the keychain service-name derivation.
  If it does, every profile silently asks to sign in again. There is no way to
  detect this ahead of a release.
- Whether a future claude adds a frontmatter key that grants capability. The
  skill store reports **any** key it does not positively recognise, so a new one
  surfaces as "declares frontmatter `x`" rather than passing silently — but the
  wording will be vague until the key is known.
