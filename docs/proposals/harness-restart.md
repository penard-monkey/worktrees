# Restarting agent sessions after a CLI update — findings and design (2026-11-03)

An update changes the executable on disk; a running agent keeps the code it
loaded until it restarts (`harness-updates.md`). This adds, under each tool's
row in Settings → Updates → Agent CLIs, a way to restart the sessions
worktrees launched so they pick up the installed version, each resuming its
own conversation.

The owner's decisions (2026-11-03): two actions, **Restart inactive** (idle
sessions only) and **Force restart all** (busy and waiting ones too, confirmed
separately with a stronger warning); scope is every registered project, each
`(main)` included.

## Measurements

All on a private tmux server (`tmux -L wt-restart-e2e-20261103-b`, every
call), a scratch repo under `/tmp`, scratch agents started with
`DISABLE_AUTOUPDATER=1`. No real session was touched.

### How a pane-0 agent is launched today

`ops::launch` creates pane 0 as

```
exec "${SHELL:-/bin/sh}" -ic '<env prefix><command>; exec "${SHELL:-/bin/sh}"'
```

`AiLaunch::pane0_body_for` builds the inner body: the profile's env
assignments (`CLAUDE_CONFIG_DIR=…`), then `launch_cmd(session)`, then the
keep-alive shell. `ops::ai_launch_for` fills the profile, guidance, the
planning plugin, the `claude-prompt` plugin and the place env; the adapter's
`launch_args` adds Claude's `--name <full tmux session>` (the messaging bus)
and Codex's `-c forced_login_method=chatgpt -c project_doc_fallback_filenames=…
--no-alt-screen <permission flags>`. The agent is a CHILD of the pane's shell,
not its exec: when it exits the pane becomes a shell.

On `open`, Claude's resume word is bare `-r`, which opens Claude's resume
PICKER; Codex's `prepare` appends `resume '<uuid>'`, the newest user thread in
the place (`codex::resume_id`).

### The restart mechanism: `respawn-pane -k` on the agent's pane

`tmux respawn-pane -k -t %N -c <place> '<pane 0 command>'` kills the pane's
processes and runs the command in the SAME pane. Measured: the pane id, the
session name, sibling panes and other windows, and attached clients are all
kept; only the pane's pid changes (72291 → 72370 in a three-pane, two-window
session). `close` + `open` would end the whole session (every pane, every
window), so it is not used.

### Claude: exact resume by `--resume <sessionId>`

A scratch Claude 2.1.316, named with `--name wt-restart-claude`, answered one
turn (`restart-proof-20261103-7c4a`). Respawned with the installed 2.1.317 and
`--name wt-restart-claude --resume beba5e74-…`: same pane `%0`, same session,
the earlier turn on screen, and its probe (`~/.claude/sessions/<pid>.json`)
reported the SAME `sessionId`, `version: 2.1.317`, `status: idle`. So a
restart resumes the probe's `sessionId` exactly, never bare `-r`.

The probe's `version` is the running Claude's version, rewritten by the new
process: it is the version source for Claude.

### Codex: exact resume by `resume <uuid>` — but the rollout keeps the OLD version

A scratch Codex 0.159.2 (an older VS Code extension binary) answered one turn.
Respawned with the PATH codex 0.160.1 and worktrees' own flags plus
`resume 01a1a368-9e3d-…`: same pane `%1`, the header box said
`OpenAI Codex (v0.160.1)`, the earlier turn and reply were on screen. It
started although Codex's weekly allowance was at 100%; no prompt was sent.

**Codex appends to the thread's existing rollout on resume.** The same file
grew from 15 to 16 lines (`event_msg/thread_settings_applied`, which carries
the model and permissions, not a version); no new `session_meta` was written,
and the first line still says `cli_version: 0.159.2`. A rollout's
`cli_version` is the version that STARTED the thread. Every worktrees `open`
resumes, so it is the wrong source for "which version runs now": it would call
a restarted session old forever, and a restart could never be proved.

What the measurement offers instead:

- `lsof -p <codex pid>` shows codex holds its rollout file OPEN. The rollout a
  process of the pane's foreground job holds is the conversation it runs,
  exactly — better than "the newest rollout for the place", which a
  `/resume` inside Codex can make wrong.
- The process's own start time and executable. npm writes extracted files with
  an mtime of 1985-10-26, so the install time is the file's **ctime**: an
  install (npm, Homebrew, a native updater, an extension unpack) creates or
  replaces the file, which sets ctime.
- On macOS `ps -o comm` is argv[0] (`codex` when launched by PATH), so the
  executable path comes from `proc_pidpath`; on Linux from `/proc/<pid>/exe`.

### kimi and pi

Neither is installed on this machine, so neither restart was measured. They
are left out: the rows answer only for Claude and Codex, and the action
refuses the other two by name.

### Cost

One read row call: one `list-panes -a`, one Claude probe scan, one
`ps -t <codex ttys> -o tty=,pid=,pgid=,tpgid=` (only when a Codex lane is up),
and per Codex lane a few `proc_pidinfo`/`/proc` reads. The installed
version is `--version` of the PATH executable, cached by (canonical path,
ctime, size), so it is spawned once per install. Activity is read only for
the sessions found older. Measured in the implementation notes below.

## Design

### Which sessions

Agent panes in worktrees places only: for each registered project the caller's
scope reaches (`projects::read_projects`), each place (`Project::place_index`,
`(main)` included), the place's live agent sessions by the agent rows' own rule
(`agents::live_agent_sessions`, i.e. `Project::live_session_in` and
`workspace::agent_session_names`) and, in each, the pane that is IN the place
and runs the harness (`tmux::pick_agent_pane`'s rule, over the one
`list-panes -a` snapshot).

- **Claude:** the lane's probe by PANE first (`agents::lane_probe`, after
  `agent::locate`), never by `cwd`. Older iff the installed version is newer
  than the probe's `version`.
- **Codex:** the foreground job of the pane's tty. Older iff its leader
  started before the installed executable's ctime, or runs a different
  executable than the installed one. Its version is shown when it is the
  installed one; an older one is "an earlier build". Its conversation is the
  rollout a process of that job holds open; none held open ⇒ not restartable
  ("could not tell which conversation it runs"), never a guess.

**Outside worktrees** (VS Code, a plain terminal, another tmux pane): Claude
probes of live pids that no place's lane owns, with an older `version`. They
are counted and listed as "restart by hand", never touched. Codex sessions
outside worktrees have no probe and are not counted: finding them would mean
reading every process's open files.

### Activity

From the SAME derivation the nav dots and `place_status` use: the harness
adapter's `activity` (`Claude::activity` = the probe's state plus the pane for
a `waiting`/missing probe; `codex_activity` = the rollout's turn plus the pane
for a modal). `idle` is inactive; `busy` and `waiting` are active; anything
else (`unknown`) counts as busy.

### Rows

`harness_sessions {tool}` — `Engine [read, global]`. Live, no cache:

```json
{ "tool": "claude", "supported": true, "installed_version": "2.1.317",
  "sessions": [ { "project": "/…/repo", "project_name": "repo", "slug": "(main)",
                  "session": "repo-main", "version": "2.1.316",
                  "state": "idle" | "busy" | "waiting", "restartable": true,
                  "why": null } ],
  "outside": [ { "version": "2.1.316", "where": "VS Code" } ] }
```

`restart_harness_sessions {tool, force}` — `Engine [admin, global]`, like
`update_harness`: a paired device needs an `admin` grant and step-up. `tool`
is a fixed enum (`claude`, `codex`; `kimi`/`pi` are refused by name). It
gathers the same lanes, then place by place under the place's agent lock
(`Agents::lock`, the lock answers and sends hold):

1. re-reads the lane (a fresh snapshot): still the same process, still older,
   and its activity NOW. Without `force` anything but `idle` is skipped
   ("turned busy since the list was read");
2. rebuilds the pane-0 command through `open`'s own path — `ai_launch_for`,
   the adapter's `prepare`, `AiLaunch::pane0_body_for` — with
   `resume: true` and the exact id: Claude's adapter emits
   `--resume '<sessionId>'` for a launch that carries `resume_id` (never
   `-r`), Codex's `prepare` its `resume '<uuid>'` as it always has. Both ids
   are checked to be UUIDs before they reach argv;
3. `respawn-pane -k -t %N -c <place>` through `tmux::tmux()` (so `-N`);
4. proves it: Claude's probe for that pane with the same `sessionId` and the
   installed `version`; Codex's pane running a codex that started after the
   respawn and holds the same rollout open. Up to 20 s, else `unconfirmed`.

The answer is per session: `restarted` (with the version proved),
`skipped` (and why), `failed` (and why), plus the `(main)` sessions the force
path restarted. A restart that a harness's `prepare` refuses (a spent window,
an untrusted folder) is `failed` with its reason; nothing was respawned.

**It never restarts the session it runs inside.** A lane's MCP server has no
such tool; a lane could still reach the row by connecting to serve as the
console with serve.json's token. So a pane whose process tree contains serve
itself is skipped (`serve runs in this session`). The caller's own pane is
not known to serve (no peer pid on the socket) — the proposal records that
gap rather than claiming more.

The audit records `tool`, `force` and the counts (restarted, skipped,
failed). Never a session id, a path or any conversation text.

### Frontend

Under each tool's row in Settings → Updates → Agent CLIs, only when older
sessions exist:

- "N Claude sessions run 2.1.316; 2.1.317 is installed." (Codex: "started
  before 0.160.1 was installed" for an earlier build.)
- **Restart N inactive** — two-click confirm: "This restarts N open Claude
  sessions. Each resumes its conversation."
- **Force restart all N** — a separate two-click confirm: "This also
  interrupts M sessions mid-turn and cancels K open questions", naming any
  `(main)` among them.
- a result line per session, and "restart by hand" for the outside count;
- refreshed after an Update and after a restart.

## Threat model

A paired device with an `admin` grant and step-up can now restart every
worktrees-launched Claude and Codex session on the host, in every project its
scope covers, `(main)` included. The force path interrupts turns and cancels
open questions. That is the owner's decision; the mitigations:

- the action is `admin` + step-up, audited with its counts;
- `tool` is a two-value enum; `force` a bool. The device names no session,
  pane, path or command: the host finds the panes, and the command is the
  one `open` builds;
- the resume ids come from the host's own reading (a probe, an open rollout)
  and are refused unless they are UUIDs;
- a restart keeps the conversation and the place's launch stamps
  (`launched_by` stays the original launcher's, so a device restart neither
  widens nor narrows the lane's MCP reach);
- sessions outside worktrees are never touched, only counted.
