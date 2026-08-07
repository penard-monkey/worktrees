# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [0.9.0] - 2026-08-06

### Added
- **AI profiles — per-project Claude rules, skills, MCP servers and model.** A
  profile is a named bundle of rules text, skills, MCP servers, settings and
  model, applied to the `claude` session worktrees launches in a worktree's tmux
  pane. Your normal terminal `claude` is untouched. A profile is either the
  global default or bound to one project, and a project profile REPLACES the
  default rather than merging with it — merging two rule sets produces a third
  one nobody wrote. `WORKTREES_PROFILE=none` opts a single launch out, and
  `worktrees open` from a terminal resolves the same profile the app would:
  one resolver in core, so the CLI and the app cannot drift. The mechanism is a
  `CLAUDE_CONFIG_DIR` swap, verified against claude 2.1.220 — rules ship via
  `--append-system-prompt-file`, MCP via `--mcp-config` plus
  `--strict-mcp-config` when not inheriting globals, which is what lets a
  profile REMOVE a global server. Each profile holds its own credential through
  a one-time `/login` in its pane, because claude derives its keychain service
  name from the config-dir path; worktrees never copies, reads or stores a
  token, which is also why deleting a profile reports the keychain item it
  cannot remove instead of pretending to have cleaned it. It **fails closed**:
  a profile that cannot be materialized opens the pane on a plain shell with
  the reason and does not launch claude, because profiles are usually
  restrictive and "could not apply your profile" must never quietly mean "ran
  without your restrictions". Stated in the UI and in DESIGN.md: a profile
  controls user scope, not the project's — a repo's committed
  `.claude/settings.json`, `.mcp.json` and `CLAUDE.md` still load, and
  `~/.claude/CLAUDE.md` with its @-imports loads regardless, so profiles ADD
  rules and cannot suppress yours. Binding a profile to a repo that already has
  conversations starts a fresh one, since history lives with the profile;
  nothing is deleted, and unbinding brings it back.
- **A skill store (`worktrees skills`)** that treats an installed skill as
  instructions the model will read: capability-shaped frontmatter is surfaced
  for review before install, git installs are pinned to the reviewed commit and
  refuse if the branch moved underneath them, and installing never executes
  anything from the source.
- **`worktrees mcp`, a hand-rolled stdio MCP server** exposing the worktree
  model to a session. Read-only by default; worktree mutations are opt-in per
  profile and the destructive ones are additionally confirm-gated. Hand-rolled
  because rmcp would have taken the CLI from 32 to 124 crates and dragged tokio
  into a sync binary.
- **The usage bars say when the window resets.** Every row in the nav footer
  gains a countdown column: `5h  ▬▬▬  35%  3h 02m`, and the weekly rows read
  `2d 5h` — days and hours, minutes dropped at that scale. The model-scoped
  bucket (Fable) gets the same treatment; a percentage alone never said whether
  it was worth waiting out. Two units, biggest first (`<1m` / `47m` / `3h 02m` /
  `2d 5h`), tabular figures so the column can't twitch as it ticks. The
  countdown runs off the local clock at 15s — `resets_at` is absolute, so the
  rate-limited endpoint keeps its 180s poll untouched. A window whose reset has
  already passed (the statusline snapshot is often that stale) shows a blank
  cell rather than a negative one, and the absolute reset time stays in the
  tooltip.
- **`doctor` now reports files the config never learned about.** Every other
  check is judged *by* `.worktrees.toml`, so a config that stopped being true was
  invisible to all of them: detection ran exactly once, at `init`, and the
  passive hint on `new` only fires for repos that have no config at all. A
  credential added the day after the config was written was therefore
  undetectable, and every worktree silently lacked it. The new `undeclared`
  finding asks the reverse question — what is gitignored, untracked, and named by
  no `[[file]]` entry. Warn for a credential, Info for `.env*`; repo-scoped, so
  it is said once and not once per worktree. It exits 0 by default (drift is the
  steady state); `doctor --strict` promotes the credential warning to a failure,
  alongside `copy-stale`. An undeclared `.env*` stays Info and never fails a run
  — the same asymmetry `--strict` already has with `copy-stale`.
- **`worktrees init --diff`** prints just the `[[file]]` stanzas an existing
  config is missing, as an appendable fragment on stdout — the second look the
  flow never had. `init` refuses to run over an existing config and `--force`
  re-renders from scratch, which destroys every hand-set `mode = "copy"` and the
  comment explaining why. This writes nothing: paths the parser refuses come out
  commented with the reason, exactly as `init` emits them.
- Adding a folder that isn't a git repository now offers `git init` plus an empty
  first commit, instead of refusing with "Not inside a git repository." A repo
  that has no commits yet is spotted in the nav too: the new-worktree form is
  replaced by a "Create initial commit" action, because git cannot branch off an
  unborn HEAD.

### Changed
- **The nav tree now ranks the project above its places.** With one project open
  the header's position carried it; with several, the eye found `★ bug-fixes`
  before it found the repo that owns it — the project name was literally smaller
  type than its own children (13px against a 15px slug, same colour, no rule
  between projects). The name now matches a slug's size at bold weight, projects
  are separated by a hairline, and the header **sticks to the top of the nav**
  while you scroll its places, so a long PINNED list can't orphan its rows.
- **Dormant recedes by fading, not by banding.** The dark full-bleed rectangle
  was darker as designed, but a hard-edged rect reads as a divider rather than
  as depth: it cut across the tree and, sitting last in each project, doubled as
  a false separator competing with the next project header. The group now sits
  at 62% opacity with no fill, and comes back to full on hover, on keyboard
  focus, and whenever the place you're standing in lives inside it. Its caret is
  the same SVG chevron every other group header uses.

### Fixed
- `worktrees new` in a repo with no commits used to fail with git's own riddle
  (`fatal: not a valid object name: 'main'`). It now refuses up front, naming the
  cause and the one command that unblocks it.

## [0.8.0] - 2026-08-03

### Added
- **The dock's Files tab reads documents properly.** Markdown renders (headings,
  nested and task lists, GFM tables, blockquotes, fenced code, links, relative
  images) with a Preview/Source toggle; source files get syntax highlighting and
  a line-number gutter; images show inline over a checkerboard with their
  dimensions, byte size and a fit/1:1 toggle; PDFs, archives, fonts and media
  get a named placeholder with "Open in editor" and "Reveal" instead of a bare
  "binary file".
- **The Files tab lays out to fit.** Past ~620px of dock width the tree moves
  beside the content instead of above it; the divider drags and the ratio
  persists. A header button cycles auto → stacked → side-by-side.
- **Reading mode (⌘⇧E).** The open file expands over the main pane at a proper
  reading measure; Esc or the Collapse button returns. The dock falls back to
  showing just the tree while it is up.

### Changed
- **The dock's file viewer is read-only.** Editing was a plain `<textarea>` with
  a save path; it is now a renderer, and edits go through "Open in editor". This
  removes the save-conflict UI and any chance of the dock clobbering a file the
  agent in the next pane is writing. (`write_file` remains in the backend.)

## [0.7.0] - 2026-08-01

### Added
- **Claude plan usage in the nav footer.** Three hairline bars mirror Claude
  Code's `/usage` panel: the 5-hour session window, the weekly all-models
  window, and any model-scoped weekly bucket (e.g. "Fable"), colored by the
  severity Anthropic reports (normal / warning / exceeded), with reset times in
  the tooltip. Data comes from the same endpoint the `/usage` panel uses,
  authenticated with the Claude Code login already in the macOS Keychain — the
  first fetch may show one Keychain prompt. If that's unavailable the app falls
  back to the statusline snapshot in `~/.claude/widgets/rate_limits.json`
  (rendered dimmed), and with no source at all the widget simply stays hidden.
- `.nvmrc` (22.13.0 — the floor pnpm 11 requires). `make install-app` checks the
  active Node against it up front rather than letting pnpm fail with its own
  version error minutes into the cargo build.

### Fixed
- Removing a place from the app always failed with `invalid args 'delBranch' for
  command 'remove_place'` and never reached the CLI. The frontend passed the flag
  as `del_branch`, but Tauri renames Rust snake_case parameters to camelCase
  across the IPC boundary. The mock harness now rejects the wrong spelling the
  same way the real backend does — it previously ignored the flag entirely, which
  is why the bug survived headless testing.
- The "Not configured" banner no longer sticks for the rest of the session once a
  `.worktrees.toml` appears from outside the app (a merge, a pull, or the CLI's
  `init`). The suggestion probe rides the same five-minute sweep as doctor instead
  of running once per project at startup.
- **AI profiles.** Settings → AI profiles lets you define what a worktrees-launched
  `claude` runs with — rules text, skills, MCP servers, model and settings — instead
  of your global `~/.claude` setup. Your normal terminal `claude` is untouched. A
  profile can be the global default or bound to one project (a project profile
  replaces the default; the two do not merge), and `WORKTREES_PROFILE=none` opts a
  launch out entirely. `worktrees open` from a terminal applies the same profile the
  app would — the CLI and the app share one resolver, so they cannot drift.
- **Each profile signs in once, on its own.** claude keys its saved sign-in to the
  config directory it is given, so a profile's first launch shows
  `Not logged in · Run /login` in the pane and stays signed in afterwards. worktrees
  never copies, reads or stores a credential to make this work. The profile list
  labels a never-launched profile "needs sign-in" so that first pane does not read
  as broken.
- **A skill store.** `worktrees skills add <dir|--git URL>` (and the same in the UI)
  installs Claude skills that profiles can enable. Because an enabled skill's
  description is loaded into every session before anything invokes it, installing one
  is closer to running someone else's prompt than to copying a file — so anything it
  asks for beyond reading files (`allowed-tools`, `hooks`, executables) is shown
  before it lands, git installs are pinned to the reviewed commit and refuse to
  install if the branch moved, and installing never runs anything from the source.
- **`worktrees mcp`** — an MCP server over stdio, so a Claude session can drive the
  worktree layer as tools instead of raw git. Read-only by default; worktree
  mutations are opt-in per profile and removing a worktree additionally requires
  explicit confirmation.
- **A "restart to apply" badge** on a live session whose profile has been edited
  since it started. It covers rules, model, MCP and settings — skill edits already
  reach a running session, so it does not claim them.

### Notes
- A profile controls the USER scope, not the project scope: a repo's own committed
  `.claude/settings.json`, `.mcp.json` and `CLAUDE.md` still load. Your global
  `~/.claude/CLAUDE.md` also still loads — a profile ADDS rules, it cannot suppress
  your own.
- Binding a profile to a repo that already has Claude conversations starts a fresh
  one, because history lives with the profile. Nothing is deleted; unbind and the
  old conversation comes back. The picker says so before you choose.
- If a profile cannot be prepared, the pane opens on a plain shell with the reason
  rather than launching claude without it — profiles are often restrictive, and
  "could not apply your profile" must never quietly mean "ran without it".

## [0.6.0] - 2026-08-01

### Added
- **Terminal tabs can be named.** Double-click a dock terminal tab to rename it
  (Enter saves, Esc cancels). Names belong to the place and survive quitting the
  app — the shell itself doesn't, so a named tab comes back as a fresh shell
  under the same name. Closing a tab forgets its name.
- **The nav's tree guide lines are optional.** Settings → Navigation → "Tree
  guide lines" turns the 1px rails connecting a project to its places off;
  indentation still carries the depth.
- **The app says when tmux is missing.** A banner above the top bar names the
  problem (`brew install tmux` on macOS) instead of leaving every place looking
  dead for no stated reason. Its `Re-check` button re-resolves the app's PATH,
  so a tmux installed while the app was open is picked up without a restart —
  the places refresh and sessions light up on the spot.

### Changed
- **Settings has categories now.** The sheet's single 12-section scroll became
  eight categories behind a category rail (Appearance, Terminal, Navigation,
  Commands, Behavior, Updates, Data & Logs, Shortcuts), and the sheet is wider.
  Every setting kept its behaviour — it just has an address now. The Updates
  category shows the update badge on its rail entry.
- The Logs tail pane in Settings is substantially taller — 200 lines of tail in
  a pane that showed eight of them was a reading slit, not a log view.
- **The installer now requires tmux.** A place *is* a tmux session, so
  installing without it produced a half-working tool. `install.sh` stops when
  tmux is absent: on macOS it offers to run `brew install tmux` (with a terminal
  attached to ask on), and on Linux it prints your distribution's exact install
  command. The CLI's own runtime behaviour is unchanged — `new` still degrades
  to `--no-tmux` if tmux disappears later.

### Fixed
- **Creating a place from the app now opens a single pane, like reopening one.**
  New places came up with the AI pane squeezed next to a spare shell, while
  reopening the same place gave Claude the full width — the same place looked
  different depending on how you got there. `new` learned `--no-spare` (which
  the app passes) so both paths agree. Dependencies are no longer auto-installed
  in that second pane; install them in the dock's Terminal tab — the command
  that would have run is printed for you. The CLI is unchanged: a bare
  `worktrees new` still splits the spare shell and installs deps there.
- The app's PATH fixup now always appends the standard install dirs
  (`~/.local/bin`, `~/bin`, `/opt/homebrew/bin`, `/usr/local/bin`) after the
  login shell's PATH, not only when the shell probe fails. A profile that never
  runs `brew shellenv` used to hide a brew-installed tmux from the GUI app.

## [0.5.0] - 2026-07-29

### Added
- **Right activity rail.** The dock's Files/Terminal picker now lives in its own
  rail on the right edge, mirroring the lens rail on the left. Clicking the
  active icon collapses the dock, so the toolbar's panel button is gone. The
  rail is always there — with no place selected its icons explain why they're
  unavailable rather than disappearing.
- **Branch switcher offers the repo's branches.** The status-bar field became a
  combobox: filter as you type, ↑/↓ and Enter to pick, and a `create <name> off
  <base>` row when what you typed doesn't exist yet. Remote-only branches are
  listed too (picking one tracks it). Typing a name and pressing Enter still
  works exactly as before.

### Changed
- **Dock shells no longer run under tmux.** Each Terminal tab is now a login
  shell this app owns directly, which means C-b reaches your shell instead of
  tmux, scrollback works with the mouse wheel instead of copy-mode, and the tab
  works even without tmux installed. Shells survive closing the dock, flipping
  tabs and switching places — their output is replayed when you come back — but
  they no longer survive quitting the app, and they are no longer
  `tmux attach`-able from a bare terminal. A shell that exits keeps its tab and
  offers a restart. The place's own session is unchanged: still tmux, still
  durable, still attachable. Leftover `~term` sessions from previous versions
  are cleaned up automatically.
- **Icons are a real set.** The chrome's Unicode glyphs (which picked a
  different font each, so weights and sizes disagreed) are now inline SVG. The
  two panel toggles say which panel they act on, and the dock toggle is no
  longer visually identical to the Places lens.

### Fixed
- The dock was capped at 680px, stranding most of a fullscreen window; its width
  now scales with the window. Side panels also re-fit when the window resizes, so
  a window restored smaller than the one your widths were saved from no longer
  overlaps its own toolbar — the dock steps aside and returns when there's room.
- The nav's ↑/↓ arrows now measure against the repo's base branch (origin/main)
  instead of the branch's own upstream. Updating a branch from main now reads as
  in sync; previously a pushed branch that merged main in showed hundreds
  "ahead" (the merged commits counted as unpushed), and branches with no
  upstream showed no arrows at all. The (main) row still works as a pull
  counter.
- Manual sort ("drag rows") actually drags now — Tauri's native drag-drop
  handler was intercepting HTML5 drag-and-drop in the app window.

## [0.4.0] - 2026-07-28

### Added
- Right dock (⌘J, or the panel button in a place's toolbar): a collapsible,
  resizable side panel with two tabs. **Files** browses the worktree as a lazy
  tree (honouring .gitignore) — click any file to view it, and edit it inline
  with ⌘S to save (binary and very large files stay read-only; "Editor" still
  opens your external editor). **Terminal** runs one or more live shells
  alongside Claude — add tabs with ＋ or ⌘⇧T, close them individually, or close
  them all. Each shell is its own tmux session, so they survive app restarts,
  are `tmux attach`-able from a bare terminal, and the open tabs are restored
  from the live sessions next time. The dock's width and last-used tab persist.
- **Per-project setup (`.worktrees.toml`).** A repo can now declare, in a
  committed file, the untracked things every worktree of it needs: which
  gitignored files to link (or copy) from the main checkout, a port map so two
  stacks can run side by side, and a docker-compose project name per place.
  Creating a worktree materializes all of it. This replaces the per-repo shell
  script most people were maintaining next to this tool, and closes the failure
  it kept causing: a credential added after a worktree existed was missing from
  it, silently — an Android build with no `google-services.json` gets no push
  token and reports no error.
  - `worktrees init` inspects a repo that has never heard of the tool and prints
    the config it would write, asking before writing anything. It flags
    credential files louder than `.env`s, because those are the ones that fail
    without saying so.
  - `worktrees relink` re-applies the file plan to worktrees that already exist,
    so adding an entry doesn't strand every place you already had.
  - `worktrees provision` allocates or repairs a port slot and writes
    `.worktree.env`.
  - `worktrees doctor` reports drift — a missing link, a dangling one, a real
    file shadowing a declared link, a port slot claimed twice, a place with no
    slot at all — and exits non-zero so CI can gate on it.
  - A file that already exists where a link belongs is **reported, never
    overwritten**. The tool it replaces silently destroyed it.
  - Nothing in the config is ever executed. A cloned repo can describe its
    structure; it cannot supply a command for the tool to run.
- **Project settings sheet in the app.** Open it from a project's context menu
  to see what the project declares, a health badge with the current findings,
  and Relink / Provision buttons. Places that have drifted get a ⚑ in the nav.
  A project that qualifies for a config but doesn't have one gets a dismissible
  suggestion.

### Changed
- The app now opens a place's tmux session as a single pane (Claude only) so it
  gets the full width — the scratch shell that used to share the split moved to
  the new dock's Terminal tab. The `worktrees` CLI is unchanged: `new` still
  splits a second pane for the dependency install.

## [0.3.2] - 2026-07-28

### Added
- Quick switcher: press ⌘K anywhere — even with the terminal focused — to
  fuzzy-jump to any place across every project. Type to filter (matches slug,
  branch, project, or note), arrow keys to move, Enter to jump, Esc to close;
  open it with no query and it lists your most recent places. Works with the
  nav collapsed, and each row shows the same working/needs-input dot as the nav.

## [0.3.1] - 2026-07-27

### Fixed
- The green "working" dot now tracks whether Claude is actually working, not
  whether you recently touched the session. It read a tmux activity timestamp
  that only moves when you attach or type — so it would fade a few seconds after
  you looked away even while Claude kept going, and it lit up for plain shell
  typing. It now reads Claude's own per-session state: a green blinking dot while
  Claude is working, a steady amber dot when it's waiting on you (a permission or
  dialog prompt), and nothing when it's idle — on the nav row, the project
  folder, and the Home screen alike.

## [0.3.0] - 2026-07-27

The settings release: a Settings pane that finally does what its controls say,
keyboard shortcuts that actually fire, and a batch of silent failures made loud.

### Added
- Nav: a Home entry at the top (app logo + one-click "Open a project" on the
  Home screen), folder icons on project rows, deeper nesting with more
  generous indent, and a right-aligned age column on place rows.
- System theme: pick WHICH light/dark pair "System (match macOS)" flips
  between (Settings → Theme → Light ↔ dark pair).
- Keyboard shortcuts: ⌘, opens Settings, ⌘1 jumps Home, ⌘2 / ⌘3 / ⌘4 jump to
  Places / Recent / Attention (keyboard selection always reveals the nav,
  never collapses it), and ⌘E opens the current selection in your editor. A
  read-only Shortcuts section in Settings lists every one of them.
- Settings → Commands: a "Resume Claude conversation on open" toggle. Turn it
  off and a single click opens a fresh session; right-click then offers "Open
  with resume" for the times you want to pick up where you left off. The
  effective AI command and resume argument are shown read-only, with a
  "Reveal config file" button — the config is shared with the CLI.
- Settings → Git: "Auto-fetch origin" (Off / 5 / 15 / 60 min) keeps every
  project's ahead/behind counts and the Attention lens fresh in the
  background, hardened so a credential prompt can never hang the app. A
  "Fetch origin" right-click verb on projects does the same on demand.
- Settings → Commands: an external terminal command with a `{session}` token
  (e.g. `ghostty -e tmux attach -t {session}`) adds "Open in terminal app" to
  a place's right-click menu — hidden until you configure it.
- Settings → Startup: "Restore last place on launch" reselects the place you
  left off on (it selects, nothing more — it never auto-starts a session).
- Settings → Version: a "Release notes" button reopens the notes sheet on
  demand, showing the full released history (not just the unseen slice), and a
  "Check for updates at launch" toggle.
- Settings → Logs: "Copy diagnostics" — one offline click assembles the app
  and CLI versions, the GUI's real resolved PATH, git/tmux locations, your
  effective AI config, and the last 200 log lines, ready to paste into a bug
  report.
- Settings → Data: reveal the settings file in Finder, and a two-click "Reset
  to defaults".
- Removing a worktree now offers "Confirm remove + branch" alongside the plain
  remove. Branch deletion uses git's merged-only guard (`git branch -d`), so
  it can never throw away unmerged work.

### Changed
- "What's new" renders formatted release notes — version headers, colored
  Added/Changed/Fixed tags, unwrapped bullets, `code` spans — instead of the
  raw changelog markdown.
- The green dot now means one thing: this session is working right now (tmux
  output within the last few seconds). Idle-but-open sessions show no dot; a
  busy place also badges its project's folder icon. The purple ✦ "AI session"
  glyph is gone — it was true for nearly every place, so it said nothing.
- The topbar remove action now reads "Remove worktree…" (was "Remove
  place…"), matching the right-click menu.

### Fixed
- The "Window default" size inputs did nothing — they were saved but never
  applied to a window. Removed.
- "Remove from workspace" could fire with zero confirmation from two different
  surfaces; both now arm on the first click and remove on the second. A
  subtler leak also let an armed "Confirm remove?" survive closing the ⋯ menu
  and then fire on a single click much later — that's fixed too.
- Copy actions failed silently — a stale clipboard with no signal that
  anything went wrong. Copy failures now surface.
- Enter could quietly do nothing. A tmux session that failed to start reported
  success everywhere (UI, exit code, and log alike), and a session running
  under a non-canonical name made a live place read as down so its terminal
  never mounted. Both are now loud and visible.
- Creating a worktree for a branch that already lived in another place could
  select a place that didn't exist, leaving the pane blank. The app now
  selects the place the engine actually used.
- Creation and switch failures used to show progress-looking lines instead of
  git's real complaint; git's actual reason now reaches the error banner and
  the log.
- Editor commands containing spaces or quotes (`open -a "Visual Studio Code"`)
  now work, and the new terminal command uses the same quoting.

## [0.2.4] - 2026-07-26

### Added
- Themes: the Settings theme picker grows from one option to seven — System
  (follows macOS light/dark), Tokyo Night (the existing default), Tokyo Night
  Day (the new light mode), Catppuccin Mocha, Catppuccin Latte, Nord, and
  Gruvbox Dark. Every palette uses the official upstream colors,
  contrast-verified for readability on every surface, and the embedded
  terminal recolors to match, including a full per-theme ANSI palette so
  colored terminal output stays legible on light backgrounds.

## [0.2.3] - 2026-07-26

### Fixed
- Embedded terminal drew `…`, `✻`, spinners, and other non-ASCII glyphs as
  underscores: the app attached tmux without a UTF-8 locale (GUI apps get
  launchd's bare environment), so tmux deemed the client non-UTF-8 and
  substituted `_` for every cell without an ACS line-drawing fallback. The
  embedded client now attaches with `tmux -u`, and the app sets a UTF-8
  `LANG` at startup when none is present (also covers the tmux server when
  the app is the first tmux invocation). Reopen embedded panes to pick it
  up — session content was never corrupted.

## [0.2.2] - 2026-07-26

### Added
- Nav preferences: show/hide the Active / Idle / Dormant tiers (Settings →
  Nav tiers), and a sort control in the nav header — Last used, A–Z, or
  Manual with drag-to-reorder (order remembered per project).
- Release notes on update: the first launch of a new version shows a
  "What's new" sheet with the changes since the version you were on
  (offline — the changelog ships inside the app).

### Fixed
- Embedded terminal artifacts: tmux sized windows to the SMALLEST attached
  client and only redrew that region, leaving stale "undeletable"
  characters when a bare `tmux attach` ran alongside the app. Sessions now
  use `window-size latest` + `aggressive-resize` (session-scoped; your
  global tmux config is untouched).
- CI actions bumped off deprecated Node 20.

## [0.2.1] - 2026-07-25

The self-updating release: from here on, updates are one click inside the app.

### Added
- **App self-update**: releases ship minisign-SIGNED app bundles + a
  `latest.json` updater manifest; Settings → Version gains "Update app → vX"
  (verify → download → swap → relaunch) next to the existing "Update CLI"
  button. The ⚙ badge covers both.
- The curl installer now OFFERS the desktop app on macOS (`worktrees.app` →
  /Applications, checksum-verified, quarantine-stripped on explicit opt-in;
  `WORKTREES_INSTALL_APP=1` / `--with-app` for non-interactive). `make
  install-app` builds + installs from a clone.
- Persistent app log (`~/Library/Logs/net.casadelvalle.worktrees/app.log`):
  every op result, terminal/updater failures, frontend errors, panics, and a
  startup line with version + resolved PATH. Settings → Logs opens the folder
  or tails it. "Check for updates" now acknowledges its result.

### Fixed
- GUI-launched apps inherited launchd's bare PATH (no homebrew → no tmux):
  every place looked dead in the installed .app. The real PATH is resolved
  from the login shell at startup.
- Nav tree: nesting is now DRAWN — per-level plumb-line rails with a lit
  ancestor trail on selection, the (main) row's dot in the project header's
  dot column, a recessed Dormant band, and a tighter indent (rails carry the
  structure, slugs keep their width).
- Settings → Logs "Open folder" was silently rejected by the capability
  system (opener:default has no open-path); now reveals app.log in Finder.

## [0.2.0] - 2026-07-25

The Rust release: one compiled engine behind both the CLI and a desktop app.

### Added
- `worktrees ls --json` (also `WORKTREES_JSON=1 worktrees ls`): a machine-readable
  snapshot (`schema_version` 1) of every place — the main checkout first, then each
  worktree with live derived state (branch/detached, dirty + file count, ahead/behind
  vs upstream, tmux session up/down, last commit, install command, Claude-session
  presence, and a computed `lifecycle_effective`). The human `ls` table is unchanged.
- `worktrees close <name> [name...]` — end a place's tmux session; the worktree,
  branch, and declared state all stay (the inverse of `open`). Resolves a branch to
  its holder worktree, closes adopted sessions (a pane cwd'd in the worktree under
  another name), and `close main` targets the main checkout — unless a worktree is
  literally named `main` (the directory wins).
- **Desktop app** (Tauri, links the engine in-process): multi-project nav tree with
  lifecycle groups (Pinned/Active/Idle + a Dormant fold), embedded tmux terminals
  (attach-not-own), create/switch/close/remove and lifecycle/pin/note from the UI,
  right-click context menus (Enter, open fresh, close session, copy attach command,
  new worktree off a branch, open on GitHub, reveal in Finder, open in editor…),
  a collapsible rail-only nav (⌘B), persisted Settings (UI font scale, terminal font,
  density, nav width, editor command), auto-resume of an existing Claude conversation
  on open, live refresh, and an in-app update check (Settings → Version) that can
  update the installed CLI via the pinned-tag installer.
- Declared lifecycle store (`.worktrees.places.json`, schema-versioned plain JSON):
  saved/archived/abandoned/closed + pin + note, reconciled with live tmux state.

### Changed
- The CLI is now a compiled Rust binary (`crates/worktrees-cli`), behavior-identical
  to the original bash version (gated by the same bats suite — now 137 cases — plus
  real-tmux smokes). `install.sh` fetches a prebuilt binary per platform (macOS/Linux,
  x86_64/arm64) or builds from source with `cargo`; `make install` compiles +
  symlinks the release binary; `bin/worktrees` is a shim that runs the built binary
  from a clone. The legacy bash implementation was retired at full parity.
- Snapshot reads are parallel (bounded fan-out per place and per project) — big
  monorepos with many worktrees list in ~max(latency) instead of sum.
- tmux kill targeting is exact-match only (`-t =name`); the prefix-match fallback
  that could hit a sibling session (`api` → `api-fix`) is gone.

### Fixed
- Claude project-dir detection now mangles every non-alphanumeric character
  (matching Claude Code), so resume detection works for paths with `_` etc.

## [0.1.0] - 2026-07-12

### Added
- Initial release: `new`/`co`, `switch`, `open`, `ls`, `rm` — git-worktree-per-branch
  workflow with a tmux session per worktree (pane 0 AI CLI, pane 1 dependency install + shell).
- Configurable AI pane: `--ai` flag, `$WORKTREES_AI_CMD` (deprecated alias
  `$WORKTREES_CLAUDE_CMD`), `ai_cmd` in `~/.config/worktrees/config`; default `claude`,
  `none` for a plain shell. Resume arg configurable (`$WORKTREES_AI_RESUME_ARG` /
  `ai_resume_arg`, default `-r`).
- Namespace prefix: `$WORKTREES_PREFIX` > `.worktree-prefix` file > user config > repo dir name.
- Runs on stock macOS bash 3.2 and Linux; git ≥ 2.23; tmux optional (≥ 1.9) — `new`
  degrades to `--no-tmux`, `open` requires it.
- `install.sh` curl installer (release-pinned, checksum-verified, `~/.local/bin`) and
  `make install` (symlink) for clones.

### Provenance
Extracted from the Casa del Valle monorepo's `scripts/worktrees.sh`, minus its
docker/stack-mode and AI-question features.
