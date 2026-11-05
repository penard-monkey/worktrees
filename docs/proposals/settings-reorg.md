---
title: "Proposal — reorganising Settings"
---

# Proposal — reorganising Settings

**Status:** built on `feat/settings-reorg` (2026-11-04): phases 0, 1 and 3
and every §6.1 option; §6.2 (Maybe) and phase 4 are not. The owner kept the
(main) guard OFF in Balanced (§5.2's table said on; "Balanced is exactly
today's defaults" won), and §9's questions took this plan's defaults. Two
places the build differs from the text, because the code's order differs
from the plan's premise: Codex permissions and pi trust are host policy >
environment, so a control the environment decides is NOT drawn disabled
(a choice there wins from then on, and the page says so); only agent
guidance's switch, where the environment wins, is. And the getters report
their source additively (`get_codex_permissions {detail}`, `pi_status
.trust_origin`, `agent_guidance_status.enabled_origin`), so no older client
gets a shape it does not read. The owner's sandbox review then renamed the
Remote group's heading to *Remote access* (its page stays *Remote*), set the
headings apart from their pages, and moved the mode switch to the header's
right. The plan was written from the code on `main` at 81a64fc0 (v0.47.0 plus #603/#604),
the mock harness (every category screenshotted), the proposals under
`docs/proposals/`, ROADMAP.md, and PrusaSlicer's knowledge base.
**The ask:** "plan a reorg of the Settings pane. suggest new categories to
organize settings into and based on the current feature set, suggest new
options or configurations we should add. Also consider introducing default
configuration sets (Beginner / Advanced / Expert), similar to how PrusaSlicer
organizes its configuration."

**Short version.**

- Settings has 19 flat categories holding settings from **seven different
  homes** (the window's blob, the host's policy file, `config.toml`, three
  JSON files under `~/.config/worktrees`, the project registry) and five
  pages that are not settings at all. The categories were added one feature
  at a time, so what belongs together sits apart (agent autonomy is spread
  over five categories) and what sits together does not belong (Appearance
  holds a polling switch; Navigation holds the layout).
- **Regroup into six groups and 17 pages** (§3): Workspace, Agents, Agent
  policy, Projects, Remote access, System. Every page shows WHICH machine a setting
  belongs to. Old category ids become aliases, so every deep link survives.
- **Modes: yes, three, named as the owner wrote them (Beginner / Advanced /
  Expert), with one rule PrusaSlicer lacks:** a setting you changed is never
  hidden, whatever the mode (§4). Hidden is never off.
- **Presets: yes, but only for agent policy** — Cautious / Balanced /
  Autonomous — because that is the one cluster where a user wants to say
  "how much may agents do" in a word and where the values live on the HOST,
  which is also why they must be distinct from modes (§5). Not whole-app
  presets: the rest of Settings is taste, and taste has no "cautious".
- **New options** (§6): the five that are clearly worth it are a Limits
  panel for the `[limits]` keys that already exist in core with no UI, the
  policy presets themselves, a confirm on the seven destructive one-click
  buttons, a "set by the environment" indicator on policy controls that the
  environment can silently override, and a Reference-panels page for the
  seven file-viewer preferences that today can only be changed from the dock
  toolbar.
- **Phasing** (§8): the regroup and the guards ship first with no migration;
  the mode switch needs one new blob key; presets need one host-policy key and
  one console-only command.

---

## 1. Inventory

### 1.1 The homes

Every setting below lives in exactly one of these. A reorg must keep the
home visible, because the home decides the scope, who may change it, and
what a downgrade does to it.

| Home | File | Owner / writer | Scope |
|---|---|---|---|
| **blob** | `<app config dir>/ui-state.json` (macOS: `~/Library/Application Support/net.casadelvalle.worktrees/`) | the frontend, WHOLE-BLOB on every change (`settings.ts` `saveSettings`) | this window. On a window driving a paired host it is the client's `remote-ui-state.json`, one file per client machine, shared by every host that client drives (`ipc.ts` `REMOTE_SETTINGS`) |
| **host policy** | `<app config dir>/host-policy.json` | serve, through five explicit setters (`worktrees-api` `hostpolicy.rs`) | the engine host; every window and every lane the host launches |
| **config.toml** | `~/.config/worktrees/config.toml` (`$XDG_CONFIG_HOME`) | serve for `[quota]`, `[slots]`, `[trust]`, `kimi_mcp`, `cross_project`; the user by hand for the rest | the machine, CLI included |
| **user JSON** | `~/.config/worktrees/{agent-guidance.json, guidance/, planning.json, profiles.json, skills.json, projects.json}` | core, through one setter each | the machine, CLI included |
| **harness config** | `~/.claude.json`, `~/.codex/config.toml`, `~/.pi/agent/mcp.json`, `<place>/.kimi-code/mcp.json` | the harness's OWN CLI (`claude mcp add`, `codex mcp add`, `pi mcp add`); worktrees never writes `~/.claude.json` | the user's harness |
| **project file** | `<repo>/.worktrees.toml` (committed) | the user, via the Project sheet's "Write" or an editor | the project, every clone. ADR 0001: never argv, never a user-only key |
| **place** | `<repo>/.worktrees.places.json` | core (`set_lifecycle`, `set_note`, `set_pin`, the status report) | one place |
| **device record** | `<app config dir>/devices.json`, `serve-remote.json`, `remote-hosts.json` | serve (host side); the shim (client side) | one paired device / the host's listener / this client |

Scope words used below: **window** (this window's blob; per device when
remote), **host** (the engine machine), **machine** (host AND its CLI),
**project**, **place**, **device**.

"Device?" says whether a window driving a paired host can change it:
**own** (it writes its own blob), **admin** (needs the `admin` capability),
**console** (refused for every device; `HostConsole` row), **—** (not a
setting). Tier is the mode this plan assigns (§4): **B**eginner,
**A**dvanced, **E**xpert.

### 1.2 Every control in the Settings sheet today

Category → section → control. "Key" is the blob key or the command the
control invokes.

| Today | Control | Key / command | Home | Scope | Device? | Default | Tier |
|---|---|---|---|---|---|---|---|
| Appearance | Overall size | `app_zoom` (⌘+ ⌘− ⌘0) | blob | window | own | 1 (100%) | B |
| Appearance | UI font size | `ui_rem` | blob | window | own | 15 | A |
| Appearance | Theme | `theme` | blob | window | own | tokyo-night | B |
| Appearance | Light ↔ dark pair | `theme_light`, `theme_dark` | blob | window | own | tokyo-day / tokyo-night | A |
| Appearance | Density | `density` | blob | window | own | comfortable | A |
| Appearance | Usage meter placement | `usage_place` | blob | window | own | strip | A |
| Appearance | Claude / Codex / Kimi plan usage | `usage_claude`, `usage_codex`, `usage_kimi` (off stops the poll) | blob | window | own | true ×3 | A |
| Terminal | Terminal font | `term_family` | blob | window | own | SF Mono, Menlo… | B |
| Terminal | Terminal size | `term_size` | blob | window | own | 13 | B |
| Terminal | Keep each tab's output between restarts | `set_term_history_opts {persistScrollback}` → `term_persist_scrollback` | host policy (mirrored to blob after an accepted set) | host | admin | true | A |
| Terminal | Give each tab its own command history | `set_term_history_opts {perTabHistory}` → `term_per_tab_history` | host policy (mirrored) | host | admin | true | A |
| Navigation | Sidebar: Pinned / Auto-hide / Hidden | `nav_pinned` + `nav_hover_reveal` | blob | window | own | unpinned + hover (auto-hide) | B |
| Navigation | Layout (four tiles) | `places_side`, `dock_side` | blob | window | own | left / right (Classic) | B |
| Navigation | Reset layout (+ Undo) | `LAYOUT_KEYS` = `places_side`, `dock_side`, `dock_lower` | blob | window | own | — | B |
| Navigation | Terminal: in the panel's tabs / below the reference panel | `dock_lower` | blob | window | own | null | A |
| Navigation | Nav width | `nav_width` | blob | window | own | 300 | A |
| Navigation | Afterglow: fade out over / steps | `done_horizon_secs`, `done_steps` | blob | window | own | 12 h / 3 | E |
| Navigation | Nav tiers (active / idle / dormant) | `hidden_tiers` | blob | window | own | [] (all shown) | A |
| Navigation | Tree guide lines | `nav_guides` | blob | window | own | true | E |
| Commands | Default agent | `default_provider` | blob | window | own | claude | B |
| Commands | Worktrees tools for agents (status + 4 "Configure" buttons) | reads `mcp_status`, `codex_mcp_status`, `pi_mcp_status`, `kimi_mcp_status` | — | — | — | — | B |
| Commands | Editor (detected list or custom) | `editor_cmd` | blob | window | own | code | B |
| Commands | Terminal command | `terminal_cmd` ("" hides the menu item) | blob | window | own | "" | A |
| Commands | Resume agent conversation on open | `ai_auto_resume` | blob | window | own | true | B |
| Commands | AI command · resume (read-only) + Reveal config file | reads `get_ai_config` (`ai_cmd`, `ai_resume_arg`) | config.toml / env | machine | — (hostOnly reveal) | claude | E |
| AI profiles | Profile list, New profile, Clear default, Make default, Delete (two-click) | `save_profile`, `set_default_profile`, `delete_profile` | user JSON `profiles.json` | machine | console (delete: admin) | no default | A |
| AI profiles | Name, Rules, Model | `Profile.name/.rules/.model` | user JSON + `~/.local/share/worktrees/profiles/<id>/` | machine | console | "New profile", "", null | A |
| AI profiles | Skill checkboxes, Remove (two-click), Add from folder…, Git URL → Review… → Install | `.skills[]`, `skill_remove`, `skill_install_local`, `skill_install_git` | user JSON `skills.json` + `~/.local/share/worktrees/skills/` | machine | console / admin | [] | A |
| AI profiles | Also load my global ~/.claude/skills | `.inherit_global_skills` | user JSON | machine | console | false | A |
| AI profiles | Expose the worktrees MCP server / …with its mutating tools too / Also load my global MCP servers | `.worktrees_mcp`, `.worktrees_mcp_mutations`, `.inherit_global_mcp` | user JSON | machine | console | false ×3 | A |
| Claude | Claude MCP server: Set up / Repair / Re-install, Remove (two-click, arm never expires), Copy command | `mcp_install {repo, mutations}` → `claude mcp add -s user`, `mcp_uninstall` | harness config `~/.claude.json` | machine (user scope) | console (status: admin) | absent | B |
| Claude | Let Claude create and close worktrees | the `mutations` argument (local state, not stored) | — | — | — | true | B |
| Claude | Stop suggesting this | `offers_dismissed["mcp-server"]` | blob | window | own | — | B |
| Codex | Permissions: Ask / Auto-review / Full access | `set_codex_permissions {mode}` → `codex_permissions` | host policy (mirrored) | host | console | auto-review | **B** (policy) |
| Codex | Your skills: Link N skill(s) for Codex, Stop suggesting | `agent_link_skills` (symlinks `~/.agents/skills/<n>` → `~/.claude/skills/<n>`) | user home | machine | console | — | A |
| Codex | Codex MCP server: Set up / Repair or update, Remove from Codex (**single click**), Install Codex CLI | `codex_mcp_install {mutations}` → `codex mcp add`, `codex_mcp_uninstall` | harness config `~/.codex/config.toml` | machine | console | absent | B |
| Codex | Allow create and close tools | the `mutations` argument | — | — | — | true | B |
| Codex | Copy servers from Claude: per-row checkbox, Apply, Refresh servers | `codex_mcp_migrate {names}`, `codex_mcp_migration_plan` | harness config | machine | console / admin | — | A |
| pi | Worktrees tools: Set up pi / Repair or update, Remove from pi (**single click**) | `pi_mcp_install {mutations}` → `pi mcp add`, `pi_mcp_uninstall` | harness config `~/.pi/agent/mcp.json` | machine | console | absent | B |
| pi | Allow create and close tools | the `mutations` argument | — | — | — | true | B |
| pi | Default model | `default_models.pi` | blob | window | own | none (pi needs one) | B |
| pi | Project trust: Skip repo resources / Let pi ask | `set_pi_trust {mode}` → `pi_project_trust` | host policy (mirrored) | host | console | never | A (policy) |
| pi | Allowed repos: Allow <repo>, Revoke (**single click**) | `set_pi_allowed {repo, allow}` → `[trust] pi = [...]` | config.toml | machine, keyed by repo | console | [] | A (policy) |
| Kimi | Worktrees tools: Set up kimi, Make read-only / Allow create and close, Disconnect (**single click**) | `kimi_mcp_set {mode}` → `kimi_mcp` (materialised per place as `.kimi-code/mcp.json` at launch) | config.toml | machine | console | off | B |
| Kimi | Default model | `default_models.kimi` | blob | window | own | "" (kimi's own) | B |
| Kimi | Folder trust: Allow <repo>, Revoke (**single click**) | `set_kimi_allowed` → `[trust] kimi` | config.toml | machine, keyed by repo | console | [] | A (policy) |
| Agent guidance | Give agents the guidance at launch | `set_agent_guidance {enabled}` → `enabled` | user JSON `agent-guidance.json` | machine | console | true | A |
| Agent guidance | Stop Claude from `git worktree add` and new branches in (main) | same call, `guard` | user JSON | machine | console | false | **B** (policy) |
| Agent guidance | Edit… the skill: Save, Compare with the default, Reset to default (two-click, 4 s), Keep mine / Use the new default / Merge | `set_agent_guidance_skill {text}`, `agent_guidance_diff`, `agent_guidance_merge` | user JSON `guidance/SKILL.md` + `SKILL.base.json` | machine | console | shipped default | E |
| Agent guidance | Stop suggesting this (×2: `agent-guidance`, `agent-guidance-changed`) | `offers_dismissed` | blob | window | own | — | A |
| Agent guidance | Other projects: Off / Read / Full | `set_cross_project {level}` → `cross_project` | config.toml | machine (new sessions) | console | off | A (policy) |
| Agent guidance | "<project> is private" per project | `set_project_private {root}` → `.private` | user JSON `projects.json` | project | console | false | A (policy) |
| Planning | On for all projects / Choose per project | `set_planning_default` → `{default}` | user JSON `planning.json` (+ `.git/info/exclude` lines) | machine | admin | unset (acts as off) | B |
| Planning | Stop suggesting this | `offers_dismissed["planning"]` | blob | window | own | — | B |
| Behavior | Restore last place on launch | `restore_last` | blob | window | own | false | B |
| Behavior | Restore window on launch | `restore_window` (read by the SHIM at launch; on a remote window it writes a value nothing reads) | blob | window | own (dead remotely) | true | A |
| Behavior | Git: Auto-fetch origin (Off / 5 / 15 / 60 min) | `set_fetch_interval {mins}` → `fetch_interval_min` | host policy (mirrored) | host | admin | 0 | A |
| Behavior | Show pull requests for GitHub projects | `pull_requests` (off stops every `gh` call) | blob | window | own | true | B |
| Behavior | Show a Chat \| Terminal switch on each agent (preview) | `chat_view` (+ one-time `chat_default_v1`) | blob | window | own | true | B |
| Behavior | Plan limits: Don't start an agent on a nearly spent plan; Weekly window: refuse at | `set_quota_settings {gate, weeklyWarnPct}` → `[quota] gate`, `weekly_warn_pct` | config.toml | machine | admin | true / provider's warning | **B** (policy) |
| Behavior | Heavy jobs: per slot − / + / Reset | `set_slot_capacity {name, capacity}` → `[slots] <name>` | config.toml | machine | **console, with no GrantNote** | machine-derived, ≤4; built-in 2 | A (policy) |
| Hosts | This window drives: Drive this machine / Drive <host> | `remote_choose {hostId}` → `chosen` | device record `remote-hosts.json` (client) | this client | own (client-local) | null (this machine) | B |
| Hosts | Paste a new address → Use this address; Forget (two-click) | `remote_readdress`, `remote_forget` | device record (client) + `remote-keys/<id>.key` | this client | own | — | A |
| Hosts | Pair with a host: link, this machine's name, Pair | `remote_pair {link, name}` | device record (client) | this client | own | hostname minus `.local` | B |
| Remote access | Turn on… (address picker) / Turn off (confirm; the kill switch) / Remove the file | `remote_enable {address, iface}`, `remote_disable` | device record `serve-remote.json` | host | console | off | A |
| Remote access | Let devices on this network find this computer by itself | `remote_advertise {on}` → `advertise` | device record `serve-remote.json` | host | console | true | E |
| Remote access | This computer's identity: Copy address code | clipboard | — | — | — | — | A |
| Devices | Pair a device…: Observe / Operate / Drive / Custom caps, Host admin, Every project / Only these, Expiry (N idle days / Until / Never), Show code, Allow / Deny | `pair_begin {caps, projects, expiry}`, `pair_decide` | device record `devices.json` | device | console | Observe, [], 30 idle days | A |
| Devices | Per device: Rename, Disable / Enable, Expiry…, Scope…, Turn step-up on/off (iOS), Revoke… (confirm) | `device_rename`, `device_disable/enable`, `device_set_expiry`, `device_set_scope`, `device_set_stepup`, `device_revoke` | device record | device | console (revoke self: own) | step-up on for iOS | A (step-up: E) |
| Updates | Check for updates automatically | `update_auto_check` | blob | window | own | true | B |
| Updates | Check for updates, Release notes, Update CLI → v, Update app → v | `update_cli {tag}`, tauri updater | — | host / this client | console | — | B |
| Updates | Agent CLIs: Update (per tool), How to update (copy), Restart N inactive / Force restart all (two-click each) | `update_harness {tool}`, `restart_harness_sessions {tool, force}` | — | host | admin | — | A |
| Updates | Install another version: Choose a version… → Roll back / Install (two-click), Install manually (copy) | `list_releases`, `install_app_version {tag}` | — | host + client | console | — | E |
| Data & Logs | Send feedback | Orfis widget (build-time keys) | — | — | — | — | B |
| Data & Logs | Logs: Open folder, View tail, Copy diagnostics | `log_info`, `log_tail`, `diagnostics {gh}` | — | host | hostOnly / admin | — | A |
| Data & Logs | Data: Reveal settings file | `settings_info` (shows the HOST's path even on a remote window) | — | — | hostOnly | — | A |
| Data & Logs | Reset to defaults (two-click) | `onReset` = `{...DEFAULTS, last_seen_version}` merged over the blob | blob | window | own | — | A |
| Data & Logs | Clear terminal history (two-click) | `term_history_clear` | `<app config dir>/term-history/` | host | admin | — | A |
| Shortcuts | (reference only) | — | — | — | — | — | B |
| Usage | Range select, Reveal file, Clear (two-click) | `ui_usage`, `ui_events_clear` | `ui-events.jsonl` (host, loopback only) | host | admin | 14 days | E |

Two notes on the table. First, `settings_rev`, `last_seen_version`,
`update_dismissed`, `chat_default_v1`, `init_dismissed` and
`offers_dismissed` are blob keys with no control of their own; they are
markers the controls above write. Second, Codex's Permissions section has
no `data-focus`, so nothing can deep-link to the one policy control most
worth linking to.

### 1.3 Blob keys that are memory, not controls

These are in `ui-state.json` with no Settings control, written by the thing
they remember. A reorg must leave them alone (and "Reset to defaults" today
does not, see §2.6). Scope is per window; on a remote window they are keyed
by the HOST's paths but stored on the client.

| Key | Written by | Scope inside the blob |
|---|---|---|
| `dock_open`, `dock_width`, `dock_tab`, `dock_lower_frac` | the dock (⌘J, the resizer, the rail) | window, and the seed for a place with no record |
| `place_panels[repo\|slug]` → `dock_open`, `dock_tab?`, `dock_width?`, `files_md_zoom?`, `files_diff?`, `agent_view?` | the dock, the Chat \| Terminal switch | place |
| `files_layout`, `files_split_pct`, `files_stack_pct`, `files_wrap`, `files_md_source`, `files_md_zoom`, `files_show_ignored`, `files_diff`, `files_diff_base`, `files_changed_only` | the Files tab's toolbar (`App.tsx:9386-9697`) | window (`files_md_zoom`/`files_diff` also per place) |
| `term_tab_names`, `term_tabs`, `term_tab_active` | the dock's shell tabs | place |
| `docs_collapsed`, `files_open`, `files_open_at` | the Docs tree, the Files viewer | place |
| `collapsed`, `manual_order`, `sort_mode`, `sort_dir` | the nav header and tree | project / window |
| `projects_parent` | the New project / Clone dialogs | window |
| `sync_with_sessions` | the Sync modal | window |
| `default_models.claude`, `default_models.codex` | the New worktree and Switch agent sheets (pi and kimi also have a Settings picker; Claude and Codex do not) | window |

### 1.4 Config with no UI at all

`~/.config/worktrees/config.toml` keys the core reads and the app never
shows:

| Key | Reader | Default | Note |
|---|---|---|---|
| `ai_cmd`, `ai_resume_arg` | `config.rs:365-394` | claude / per harness | shown read-only under Commands; ROADMAP:2414 wants an editor (needs a comment-preserving writer) |
| `prefix` | `project.rs:738` | repo basename | also `[project] prefix` in `.worktrees.toml` and `WORKTREES_PREFIX` |
| `post_create` | ADR 0001's user-scoped hook | none | the only argv a user may add; by hand only |
| `codex_permissions`, `pi_project_trust` | the tier BELOW host policy | — | the app writes host policy instead; a config.toml value shows as the effective value until the host holds a key |
| `[model] <harness>` | `config.rs:351`, used for pi and kimi launches from the CLI | none | **a second store for the same fact as the blob's `default_models`**; neither surface mentions the other |
| `[limits] max_live_agents`, `max_live.<provider>`, `min_free_memory_pct`, `max_swap_used_pct` | `guard.rs:41-138`, the launch guard | half the RAM in GiB / none / 20 / 0 | ROADMAP:914-920 already asks for a Settings → Limits |
| `[sync] hub`, `with_sessions`, `projects.<n>.install`, `extra_excludes` | `config.rs:172-231` | none | the app's own hub list is `sync-hubs.json` and its `sync_with_sessions` is in the blob; `[sync] hub` outranks both |
| `[trust] opencode` | proposed only (`opencode-harness.md` §8) | — | — |

Environment variables that override a Settings control **without the UI
saying so** (the control reads the effective value, so it looks stuck or
wrong): `WORKTREES_CODEX_PERMISSIONS`, `WORKTREES_PI_PROJECT_TRUST`,
`WORKTREES_AGENT_GUIDANCE`. Two are shown with a banner:
`WORKTREES_PROFILE` and `WORKTREES_KIMI_MCP`. The rest
(`WORKTREES_AI_CMD`, `WORKTREES_AI_RESUME_ARG`, `WORKTREES_PREFIX`,
`WORKTREES_NO_PROJECT_CONFIG`, `WORKTREES_TMUX_NAMESPACE`,
`WORKTREES_{KIMI,PI,CLI,GH,SSH}_BIN`, `WORKTREES_RSYNC`,
`WORKTREES_SYNC_HUB`, `WORKTREES_KIMI_OPENER_WAIT_MS`, `WORKTREES_NO_PROMPT`,
`WORKTREES_JSON`) are CLI and development knobs with no app surface, which
is fine; the three above are not.

### 1.5 Per-project and per-place settings that live outside the sheet

Reached from the nav, not from ⌘,. They stay where they are; §3 gives them a
Settings page that LISTS them with a link, never a second editor.

| Surface | Setting | Home | Scope |
|---|---|---|---|
| Project sheet | AI profile for this project | `profiles.json` `assignments[root]` (console-only) | project |
| Project sheet / Settings → Planning | Planning: inherit / Off / Show only (path, each place or main's copy) / Full | `projects.json` entry (admin) | project |
| Project sheet | Write / open `.worktrees.toml`; Relink; Re-seed (confirm); Provision ports | project file (`[[file]]`, `[ports]`, `[compose]`, `[project] prefix`, `[docs]`); `.worktree.env` per place | project / place |
| Project sheet | Agent setup Fix… (+ push), Rules apply… (+ push, force) | the repo's `agent-instructions` branch | project |
| Settings → Agent guidance | "<project> is private" | `projects.json` `.private` | project |
| Status sheet | Abandon / Archive (lifecycle), Remove worktree…, Ask Claude (status report) | `.worktrees.places.json` | place |
| Nav | pin, note, title, lifecycle | `.worktrees.places.json` (declared) | place |
| CLI only | project rename (`worktrees projects …`) | `projects.json` | project |

`USER_ONLY_KEYS` (`projcfg.rs:51-79`) is the fence: `ai_cmd`,
`ai_resume_arg`, `post_create`, `hooks`, `infra`, `install_cmd*`,
`no_install`, `harness`, `model`, `trust`, `pi_project_trust`, `kimi_mcp`,
`cross_project`, `quota`, `slots`, `limits`. ADR 0001's own table still
lists the first seven; the code has sixteen. Worth a one-line update when
the ADR is next touched, not a design question.

---

## 2. What is wrong today

### 2.1 Findability: 19 flat categories, one level, no search

The sheet grew one category per feature (Claude, Codex, pi, Kimi, Agent
guidance, Planning, Hosts, Remote access, Devices each arrived with a PR and
each got a row). Nineteen rows is past what a flat list can carry: the
screenshots show the nav running to the bottom of a 900-px window with
"Usage" behind a divider at the fold. Three concrete misses:

- **"Where is the Codex permission?"** Under Codex. **"Where is pi's
  equivalent?"** Under pi, called Project trust. **"Where is the thing that
  stops Claude branching in (main)?"** Under Agent guidance. **"Where do I
  cap how many agents run?"** Nowhere (`[limits]` has no UI), and the
  nearest thing, Heavy jobs, is under Behavior. The one question a user
  asks most, "how much may my agents do", has five answers in five places.
- **"Where is the dock's terminal placement?"** Under Navigation, labelled
  "Terminal", one row below a category called Terminal that does not hold
  it.
- **"Where is Chat view?"** Under Behavior, between Pull requests and Plan
  limits. It is a property of the agent cell.

### 2.2 Overloaded categories

- **Behavior** holds six sections from four homes: two blob startup
  toggles, a host-policy git interval, a blob PR switch, a blob chat
  switch, and two `config.toml` panels. Nothing in the name says which.
- **Appearance** holds the usage meter's PLACEMENT (appearance) beside three
  per-provider polling switches that start and stop network calls (not
  appearance).
- **Commands** holds the default agent, a status summary of four MCP
  servers with four "Configure" buttons that merely switch category, the
  editor, the terminal command, auto-resume, and a read-only display of the
  CLI's `ai_cmd`. ROADMAP:1500-1506 already notes "two unrelated things are
  now called 'Claude' in Settings".
- **Navigation** holds the whole window layout (tiles, dock lower slot) and
  the afterglow decay, which is a nav-dot setting but reads as decoration.

### 2.3 Settings that belong together and sit apart

| Cluster | Today | Count of categories |
|---|---|---|
| Agent autonomy and safety | Codex → Permissions; pi → Project trust, Allowed repos; Kimi → Folder trust; Agent guidance → guard, Other projects; Behavior → Plan limits, Heavy jobs; AI profiles → worktrees_mcp_mutations | 5 |
| Which agent, which model | Commands → Default agent; pi → Default model; Kimi → Default model; New worktree sheet → Claude/Codex model; `config.toml [model]` (CLI) | 3 + a dialog + a file |
| Connecting the worktrees tools | Claude, Codex, pi, Kimi (each with its own "Allow create and close" checkbox, "Stop suggesting", and verdict words), plus the status summary under Commands | 5 |
| Remote | Hosts (this client), Remote access (the host's listener), Devices (the host's pairings) | 3 |
| Terminal | Terminal (font, host-policy history), Navigation (dock placement), Commands (terminal command), Data & Logs (clear saved history) | 4 |
| Planning | Settings → Planning (global + per project) and the Project sheet (the same `ProjectRow`) | 2 surfaces, fine; but the global default is the only reason Planning is a top-level category |
| Offers' "Stop suggesting this" | nine buttons across six categories, each phrased the same | 6 |

### 2.4 Scope confusion

- The sheet draws every control the same way whether it changes this
  window, this machine, or every lane the host will ever launch. The scope
  is in prose hints ("Kept by this machine rather than this window",
  "Applies to the next Codex launch", "Takes effect for new sessions") and
  the hints are not uniform: Codex Permissions says it, Heavy jobs does not.
- On a window driving a paired host, **every blob key is per device**
  (`remote-ui-state.json`), including `default_provider`, `editor_cmd` and
  `ai_auto_resume`, which Phase 2 §9.1 meant to be shared and
  `patch_settings` would have made shared. That is not built, so "Default
  agent" on the phone and on the Mac are two different answers with no sign
  of it. `restore_window` on a remote window writes a value nothing reads.
  "Reveal settings file" on a remote window shows the HOST's path.
- Only the four MCP sections, Remote access and Devices swap in a
  `GrantNote` on a remote window. Guidance, Cross-project, Planning,
  Profiles, pi/Kimi trust, Quota, Slots and Auto-fetch render their
  controls and are refused on click (Slots with its buttons still enabled,
  reporting only through `onReport`).
- Codex Permissions, pi trust and Guidance can be overridden by an
  environment variable the control never mentions (§1.4). A control that
  reads `full` and cannot be moved off it looks like a bug.
- The default model has two stores (blob `default_models`, `config.toml
  [model]`) and two consumers (the app, the CLI). Changing one does nothing
  for the other, and neither says so.

### 2.5 Dangerous options with no guard

| Control | Today | Hazard |
|---|---|---|
| Codex → Remove from Codex | one click | drops the server entry from `~/.codex/config.toml` |
| pi → Remove from pi | one click | same, `mcp.json` |
| Kimi → Disconnect | one click | removes `kimi_mcp` from config.toml; every kimi lane loses its tools at next launch |
| pi / Kimi → Revoke <repo> | one click | removes an allowance a lane may be relying on |
| Codex → Permissions → Full access | one click, no confirm, no wording stronger than the other two | "no prompts and no sandbox" for every Codex lane the host launches from now on. The one control that widens what an agent may do on the machine is the one with no friction |
| Agent guidance → invalid edit → Reset to default | one click (the healthy-path Reset is two-click with a 4 s arm) | discards the user's edit |
| Claude → Remove; Profiles → Delete; Skills → Remove | two-click, **arm never expires** | AGENTS.md's rule ("short, so an arm forgotten does not restart anything on a stray click") is followed by Guidance (4 s) and the restart buttons (4 s) and not here |
| Updates → Install another version | two-click | fine as is |

### 2.6 "Reset to defaults" resets more than it says

The hint reads "Restores every setting to its default (theme, fonts, nav,
sort, tiers)". `onReset` (App.tsx) merges `DEFAULTS` over the blob, which
also wipes `place_panels`, `term_tabs`, `term_tab_names`,
`term_tab_active`, `docs_collapsed`, `files_open`, `files_open_at`,
`manual_order`, `collapsed`, `init_dismissed`, `offers_dismissed` and the
three layout keys. The per-place memory of every place and every silenced
offer goes with the theme. (It leaves host policy and the mirror keys alone,
which is right; the `{...prev, ...next}` merge keeps keys `DEFAULTS` lacks.)
A reset that forgets which file you had open in forty places is not a
"reset to defaults", and re-asking every offer is the v0.25.0 bug in reverse.

### 2.7 Pages that are not settings

Shortcuts is a reference card. Usage is a record ("what the app has RECORDED
rather than what it will do", and ROADMAP:1685 plans to cut it after two
weeks). Hosts' pairing form is an action. Updates is mostly actions. They
are not wrong to be reachable from ⌘,; they are wrong to weigh the same as
Appearance in a flat list.

---

## 3. Proposed categories

### 3.1 The structure

Six **groups** with a header each in the left nav, 17 **pages** (`CatId`s).
A group header is not clickable; it is what lets 17 rows read as six
things. The mode (§4) hides whole pages and sections; the groups stay.

```
WORKSPACE        this window
  General
  Appearance
  Layout
  Terminal
  Reference panels            (Advanced)
AGENTS           what runs in a place
  Agents
  Claude · Codex · pi · Kimi  (a tab shows if its CLI is installed, or in Advanced)
  Profiles                    (Advanced)
  Guidance                    (Advanced)
AGENT POLICY     this machine: how much agents may do
  Agent policy                ← presets live here (§5)
PROJECTS
  Projects
REMOTE ACCESS
  Remote
SYSTEM
  Updates
  Data & privacy
  Shortcuts
```

Every page states its scope under its title, in one of four fixed
sentences, so the hints stop repeating it per control:

- *These settings are this window's.* (blob)
- *These settings are this computer's: every window and every agent it
  launches.* (host policy, config.toml, user JSON)
- *This is the project's own file; every clone gets it.* (project file,
  shown only as a link)
- On a remote window: *Set up at `<host>` itself; a paired device may read
  it.* (the existing GrantNote wording, now at page level)

A control whose home differs from its page's sentence (there are four:
Terminal's two history switches, Agents' chat view, Remote's "this window
drives") carries a small scope chip instead ("this computer", "this
window").

### 3.2 Each page

**General** (window). Startup: Restore last place, Restore window (hidden
on a remote window, where it is dead). Commands: Editor, Terminal command.
Pull requests on/off. New projects go in (`projects_parent`, today only set
by the dialogs). Check for updates automatically moves to Updates.

**Appearance** (window). Overall size, UI font size (A), Theme + pair (A),
Density (A). Nothing that polls.

**Layout** (window). Sidebar, Layout tiles + Reset layout, Nav width (A),
Nav tiers (A), Terminal placement in the reference column (A), Usage meter
placement (A; the placement IS layout), Tree guide lines (E), Afterglow (E).

**Terminal**. Font, size (window). What a shell tab remembers (host, chip:
"this computer", A). Saved terminal history size + Clear (host, A; moved
from Data because it is the only place that explains what the history is).

**Reference panels** (window, Advanced). The seven `files_*` preferences
that today only the dock toolbar can change: layout auto/stack/split and
the two split shares, wrap, markdown source vs render, reading size default,
show ignored, changed-only, diff base. New as a Settings surface, not as
keys (§6, item 5).

**Agents** (window + status). The overview: Default agent; Default model per
installed harness (Claude and Codex gain the picker pi and Kimi already have;
the row says which store the CLI reads instead, §6 item 6); Resume agent
conversation on open; Chat view switch, and a per-harness default view row
(A); the connection table (one row per harness: tools state, a Configure
link, the plan-usage switch from Appearance beside the harness it polls).
This is the page `pi-harness.md` §2.4 and `cross-project.md` §P1b both
named "Settings → Agents" and never got.

**Claude / Codex / pi / Kimi** (machine). Unchanged in content, one shape:
Worktrees tools (set up / repair / remove, the create-and-close checkbox),
then harness-specific sections (Codex: skills, copy servers from Claude; pi
and Kimi: models). **Policy sections leave these pages** (Codex Permissions,
pi Project trust and Allowed repos, Kimi Folder trust) for Agent policy, and
each harness page keeps a one-line pointer with the current value ("Permissions:
Auto-review — change under Agent policy"), because a user who comes here
looking for it must still find it. A harness page is listed when its CLI is
installed on the host, or in Advanced mode; a Beginner with only Claude sees
only Claude.

**Profiles** (machine, Advanced). The AI profiles panel as is.

**Guidance** (machine, Advanced). Delivery on/off, the skill editor and its
compare/merge flow (E). The guard toggle moves to Agent policy; this page
keeps the pointer line.

**Agent policy** (machine). The page the presets sit on. Sections, top to
bottom:

1. **Preset** (§5): Cautious / Balanced / Autonomous / Custom, the
   "modified" marker, Reset to preset, Compare.
2. **Codex**: Permissions (Ask / Auto-review / Full access; Full access
   gains a confirm).
3. **Claude**: the (main) branching guard.
4. **pi**: Project trust; Allowed repos.
5. **Kimi**: Folder trust.
6. **Other projects**: Off / Read / Full, and the private-project list.
7. **Plan limits**: the gate and the weekly threshold.
8. **Limits** (new, §6 item 1): live agents, per provider, memory floors.
9. **Heavy jobs**: the slots panel (A).

Every section here carries the environment indicator (§6 item 4) when the
engine's environment overrides it.

**Projects** (machine). Planning default (the two buttons and the
explanation, moved from the Planning category), then a table of registered
projects: name, planning level, profile, private, a "Project settings…"
link that opens the Project sheet. The per-project controls that today live
in Settings → Agent guidance (private) and Settings → Planning (the
`ProjectRow`) become columns here; the Project sheet stays the editor for
`.worktrees.toml`, relink, provision and agent setup, as `project-settings.md`
§10 decided.

**Remote**. Three sections, each with its own scope sentence: *This window
drives* (client, B), *Let devices reach this computer* (host, console, A:
the listener, kill switch, identity, advertise at E), *Paired devices*
(host, console, A; step-up at E). On a remote window the second and third
show the GrantNote, so the page is still worth opening for the first. The
two badges (`remote`, `devices`) become one on the page and keep their
targets as focus ids.

**Updates**. Version + Check automatically (moved from General), Agent CLIs
with the restart panels (A), Install another version (E).

**Data & privacy**. Feedback, Logs, Settings file (with the RIGHT path on a
remote window), Reset (with a scope picker, §6 item 7), the Usage record as
a collapsed section at the bottom (E), replacing the Usage category.

**Shortcuts**. As is, last.

### 3.3 Migration table (old → new)

| Old category | Section | New page | Notes |
|---|---|---|---|
| Appearance | Overall size, UI font size, Theme, Density | Appearance | |
| Appearance | Usage meter placement | Layout | |
| Appearance | Claude / Codex / Kimi plan usage | Agents (connection table) | |
| Terminal | font, size | Terminal | |
| Terminal | What a shell tab remembers | Terminal (chip: this computer) | |
| Navigation | Sidebar, Layout, Reset layout, Terminal placement, Nav width, Nav tiers, Tree guides, Afterglow | Layout | |
| Commands | Default agent, Resume on open | Agents | |
| Commands | Worktrees tools for agents (status) | Agents (connection table) | the four "Configure" buttons become the table's links |
| Commands | Editor, Terminal command | General | |
| Commands | AI command (read-only), Reveal config file | Agents (E), beside Default model | with the `[model]` note |
| AI profiles | all | Profiles | |
| Claude | all | Claude | |
| Codex | Permissions | **Agent policy** | Codex page keeps a pointer |
| Codex | Your skills, MCP server, Copy servers from Claude | Codex | |
| pi | Worktrees tools, Default model | pi | |
| pi | Project trust, Allowed repos | **Agent policy** | pointer stays |
| Kimi | Worktrees tools, Default model | Kimi | |
| Kimi | Folder trust | **Agent policy** | pointer stays |
| Agent guidance | delivery, Edit…, compare/merge | Guidance | |
| Agent guidance | the (main) guard | **Agent policy** | pointer stays |
| Agent guidance | Other projects, private list | **Agent policy** (reach) + Projects (private column) | one writer, two readers |
| Planning | global default | Projects | |
| Planning | per-project rows | Projects (table) | |
| Behavior | Restore last place, Restore window | General | |
| Behavior | Auto-fetch origin | General (chip: this computer) | |
| Behavior | Pull requests | General | |
| Behavior | Chat view | Agents | |
| Behavior | Plan limits, Heavy jobs | **Agent policy** | |
| Hosts | all | Remote → This window drives | |
| Remote access | all | Remote → Let devices reach this computer | |
| Devices | all | Remote → Paired devices | |
| Updates | all + Check automatically | Updates | |
| Data & Logs | Feedback, Logs, Settings file, Reset | Data & privacy | |
| Data & Logs | Clear terminal history | Terminal | |
| Shortcuts | | Shortcuts | |
| Usage | | Data & privacy (collapsed, E) | |

### 3.4 Deep links carry over

`SettingsSheet` takes `at: {cat, focus}`; `offers.ts` has nine targets and
App.tsx three more. The rule: **old `CatId`s stay valid as aliases, forever**,
through one table in `SettingsSheet.tsx`:

```ts
/** A category that no longer exists, mapped to where its sections went.
 *  `focus` wins when the caller gave one; the alias supplies a default
 *  focus for callers that named only the category. */
const LEGACY_CATS: Record<string, { cat: CatId; focus?: string }> = {
  appearance: { cat: "appearance" },
  navigation: { cat: "layout" },
  commands:   { cat: "agents" },
  ai:         { cat: "profiles" },
  guidance:   { cat: "guidance" },
  planning:   { cat: "projects", focus: "planning" },
  behavior:   { cat: "general" },
  hosts:      { cat: "remote", focus: "this-window-drives" },
  "remote":   { cat: "remote", focus: "remote-access" },
  devices:    { cat: "remote", focus: "devices" },
  data:       { cat: "data" },
  usage:      { cat: "data", focus: "usage" },
};
```

| Caller | Target today | Resolves to |
|---|---|---|
| offers `mcp-server` | claude / mcp-server | Claude / mcp-server (unchanged) |
| offers `codex-mcp`, `codex-skills` | codex / codex-mcp, codex-skills | unchanged |
| offers `pi-mcp`, `kimi-mcp` | pi, kimi | unchanged |
| offers `agent-guidance`, `agent-guidance-changed` | guidance | Guidance (unchanged id) |
| offers `cross-project` | guidance / cross-project | **Agent policy / cross-project** (the focus id moves pages; `focus` resolves against the whole sheet, so the alias maps the focus, not the caller) |
| offers `planning` | planning / planning | Projects / planning |
| Project sheet "Default & how it works…" | planning / planning | Projects / planning |
| UpdateBubble | updates | Updates (unchanged; `onCatShown("updates")` keeps retiring the bubble) |
| remoteStore opener | remote, devices | Remote / remote-access, Remote / devices |
| Codex Permissions (no focus today) | — | Agent policy / codex-permissions (new `data-focus`) |

Two focus ids need to move with their sections and must be looked up
sheet-wide rather than per page: `cross-project`, `plan-limits`,
`heavy-jobs`, `codex-permissions`. The resolver already queries
`[data-focus]` in the body after the category paints; the only change is
that the alias table may rewrite `cat` when `focus` belongs to another page
(a `FOCUS_HOME: Record<focus, CatId>` map, drift-checked against the JSX
by a script in the shape of `offers-check.mjs`).

The offers rule in AGENTS.md ("a suggestion's surface may not add
preconditions of its own") is unaffected: the offer band is not in the
sheet. But §4's modes add one precondition to be refused explicitly: **a
deep link must reveal its target whatever the mode** (see §4.3).

---

## 4. Modes: Beginner / Advanced / Expert

### 4.1 What PrusaSlicer does, and what carries over

Verified from Prusa's knowledge base (the 2.9 articles; the current release
may differ, and the source-level tier tags could not be fetched):

- Three modes, Simple / Advanced / Expert. "In the Advanced mode, more
  parameters are shown, and in the Expert mode, all the power features of
  PrusaSlicer are accessible." Simple exists "to lower the learning curve
  for beginners and declutter the user interface for casual users".
- Each parameter is colour-coded by tier: green "safe to be tinkered with by
  a beginner", yellow "advanced users may want to modify these", red
  "expert settings most of which should only be touched if you're making a
  profile for a new printer". In the source each option carries a mode tag
  (`comSimple` / `comAdvanced` / `comExpert`, from a 2.3.3 mirror).
- The switch is a drop-down at the top right of the main window, not in
  Preferences. Preferences does not mention modes at all.
- The docs do not state the default mode or what happens to a value set in
  Expert when the user switches to Simple. In practice the mode only hides:
  presets carry every value, and a hidden value keeps applying. The known
  complaint (forum threads about "my print changed and I can't see why")
  follows from exactly that: a value changed in Expert disappears in Simple
  and keeps acting.

What translates: tiers per section, a mode that hides and never disables,
a switch outside the settings tree, colour as a tier marker (here it must be
a dot or chip, not coloured text, per the contrast rule). What does not: a
hidden value must stay visible once it is non-default, and the tier should
be per SECTION, not per control, because this sheet has ~120 controls and a
per-control tag would make the pages patchy.

### 4.2 Recommendation: yes, three modes, four rules

1. **Hidden is never off.** A mode changes what the sheet draws, never what
   the app or the host does. Every value keeps applying.
2. **A changed setting is never hidden.** A section whose tier is above the
   current mode is still drawn when any control in it holds a non-default
   value, with a chip ("Advanced setting") where its tier dot would be. This
   is the one rule PrusaSlicer lacks and the whole reason its forum has the
   "why did this change" thread. The comparison is against `DEFAULTS` for
   the blob and against the host's default for policy (the getters answer
   effective values; §4.5 says how).
3. **A problem is never hidden.** A stale MCP server, an unreadable
   `serve-remote.json`, an invalid skill edit, a device alert, a quota
   "problems" list: these show in every mode, as the offers code already
   says problems are not dismissible.
4. **An installed harness is never hidden.** Claude / Codex / pi / Kimi
   pages show when the CLI is on the host, whatever the mode.

### 4.3 What each mode shows

Tiers are in the inventory (§1.2). By page:

| Page | Beginner shows | Advanced adds | Expert adds |
|---|---|---|---|
| General | Restore last place, Editor, Pull requests | Restore window, Terminal command, Auto-fetch, New projects go in | — |
| Appearance | Overall size, Theme | UI font size, light/dark pair, Density | — |
| Layout | Sidebar, Layout tiles, Reset layout | Nav width, Nav tiers, Terminal placement, Usage meter placement | Tree guides, Afterglow |
| Terminal | Font, Size | What a shell tab remembers, Saved history + Clear | — |
| Reference panels | (page hidden) | the page | — |
| Agents | Default agent, Default model (installed harnesses), Resume on open, Chat view, connection table | per-harness default view, plan-usage switches | AI command read-only + `[model]` note |
| Claude / Codex / pi / Kimi | Worktrees tools (+ the checkbox), Default model (pi, Kimi), policy pointer line | Codex: skills, Copy servers from Claude; pages for harnesses not installed | — |
| Profiles | (page hidden) | the page | — |
| Guidance | (page hidden) | delivery switch, the two offers' "Stop suggesting" | the skill editor, compare, merge, reset |
| Agent policy | Preset, Codex permissions, the (main) guard, Plan limits gate | pi trust + allowed repos, Kimi trust, Other projects + private, Limits, Heavy jobs, Plan-limits threshold | — |
| Projects | Planning default, the project table (name, planning, Project settings… link) | profile and private columns | — |
| Remote | This window drives (Drive, Pair) | Let devices reach this computer, Paired devices, Forget / readdress | Advertise, step-up |
| Updates | Version, Check automatically, Check / Update / Release notes | Agent CLIs + restarts | Install another version |
| Data & privacy | Feedback | Logs, Settings file, Reset, Clear terminal history | Usage record |
| Shortcuts | all | — | — |

A page whose every section is above the mode is not listed in the nav; its
group header stays, and the group's last visible page gets a footer line
"**N more settings in Advanced ›**" that switches the mode in place. So
Beginner can always see that there is more, which is the second half of the
PrusaSlicer complaint (settings a beginner needs, hidden with no sign).

A **deep link** to a hidden section reveals that section for the visit (it
is drawn with its tier chip, the rest of the page stays as the mode says).
The mode does not change; a footer under the revealed section offers the
switch. Without this, every offer and every "Change under Agent policy"
pointer would need to know the mode, which is the precondition trap.

### 4.4 Why these tiers

- **Beginner** is "what you need in the first hour": look, size, which
  agent, connect its tools, how much it may do, update. It deliberately
  includes Codex Permissions and the (main) guard: a beginner is the person
  most in need of seeing that Codex defaults to Auto-review and what that
  means, and §5's presets make the choice one word.
- **Advanced** is "the shape of your workbench": layout details, host-side
  terminal behaviour, the file viewer, per-harness extras, profiles,
  guidance delivery, the trust lists, limits, remote hosting, agent CLI
  restarts. Everything a second week brings.
- **Expert** is "things that need the manual": the skill editor, afterglow
  geometry, tree guides, mDNS advertise, step-up, rollback installs, the
  usage record. Each has a hint longer than its control.

### 4.5 Behaviour across modes

- **Values set in a higher mode and then the user drops a mode:** still
  applied (rule 1) and still shown with the chip (rule 2). A "Reset to
  defaults" scoped to "settings above my mode" is offered on the Data page
  (§6 item 7), which is how a Beginner gets rid of a chip they do not
  understand without finding the control.
- **Policy values vs. the default:** `get_codex_permissions` and friends
  answer the EFFECTIVE value (AGENTS.md: not a record of what the host
  holds). For the chip the question is "is this different from the shipped
  default", which the effective value answers correctly; for the
  environment indicator (§6 item 4) the question is "who set it", which
  needs the getters to also report a source. Both are read-side changes.
- **Downgrade:** an older build ignores `settings_mode`. Nothing else
  changes shape.
- **A remote window** keeps its own mode (it is in the device's blob).

### 4.6 Where the switch lives

In the Settings header, at its right beside the esc / ⌘, hints and the
close button (the owner's review moved it there from beside the title), as a three-way segmented
control (`.seg`, the same control Sidebar uses), **not** in a category and
not in a popover (no new `useEscape` surface). PrusaSlicer puts it on the
main window; this app's main window has no chrome to spare and the mode
only affects the sheet, so the sheet's header is the honest place. The
rail's gear tooltip is unchanged.

Key: `settings_mode: "beginner" | "advanced" | "expert"` in the blob (per
window; a phone and a Mac can differ, which is right).

### 4.7 Defaults

- **Fresh install: Beginner**, with the footer lines making Advanced one
  click away. The app's first-hour story (connect Claude, pick a theme,
  decide Codex's permission) fits in Beginner.
- **Upgrade: Expert.** `migrate` rev 5: a blob that exists before the key
  gets `expert`, so nobody who has been using the sheet finds a row gone on
  the morning of the release. This is the opposite of what rev 2 did for the
  sidebar (everyone landed on the new default, deliberately) and the
  difference is that hiding is not a design the app is now built around; it
  is a convenience.
- Open for the owner (§9): whether fresh installs should land on Advanced
  instead, on the grounds that a person installing a tmux-and-git-worktree
  tool is not a beginner at anything but this app.

---

## 5. Presets: Cautious / Balanced / Autonomous

### 5.1 Recommendation: yes, for agent policy only; keep them distinct from modes

A mode says what you SEE. A preset says what you HAVE. PrusaSlicer keeps the
two apart (a system preset is a set of values; a mode is a filter over the
tabs) and so should this app, in two ways:

- Modes are per window (blob). Presets set host-side values. A preset
  chosen on a phone must change the host, and a mode chosen on a phone must
  not.
- A mode is reversible with nothing lost. Applying a preset rewrites values;
  it needs the modified marker and the reset.

Whole-app presets ("Beginner preset sets theme X") are not worth it:
appearance and layout have no cautious/autonomous axis, and bundling them
would make the preset name lie the moment a user picked a theme. The one
cluster where a user wants a word instead of nine controls is agent policy,
and §2.3 shows it is the cluster most in need of a single page.

### 5.2 What a preset sets

System presets, shipped in core (`policy_presets.rs`, versioned like
`guidance::VERSION`):

| Setting | Home | Cautious | Balanced | Autonomous |
|---|---|---|---|---|
| Codex permissions | host policy | ask | auto-review (today's default) | full |
| Claude (main) branching guard | agent-guidance.json | on | on | off |
| pi project trust | host policy | never | never | ask |
| Other projects (cross-project reach) | config.toml | off | off | read |
| Plan limits gate / weekly threshold | config.toml `[quota]` | on / 80% | on / provider's warning (today's default) | off / — |
| Live-agent limit (`[limits] max_live_agents`) | config.toml | 2 | machine default | machine default |
| Default for the "Allow create and close tools" checkbox on a Set up button | the Agents pages' local state | unticked | ticked | ticked |
| Default scope when pairing a device | the Pair sheet's initial choice | Observe | Observe | Operate |

Deliberately NOT in a preset: Heavy-job slots (machine-derived, not a
trust question), the pi/Kimi allowed-repo lists (per repo, additive, a
preset would have to clear them), `ai_auto_resume` and `chat_view` (window
convenience, blob, not safety; the owner's brief listed auto-resume as an
example and this plan recommends leaving it out, because a preset that
writes the blob would behave differently on a remote window, where the blob
is the device's). "Balanced" is exactly today's defaults, so applying it on
a fresh install changes nothing, which is what makes it safe to be the
starting preset.

### 5.3 System vs. user presets, the modified marker, reset

- **Stored as one key**, `policy_preset: {name, version}` in
  `host-policy.json`, written by one console-only command
  `apply_policy_preset {name}` that calls the existing setters in order
  (`set_codex_permissions`, `set_agent_guidance`, `set_pi_trust`,
  `set_cross_project`, `set_quota_settings`, the new `set_limits`), so every
  mirror, migration and audit record happens as it does for a hand change.
  No setter learns about presets. A partial failure (one setter refused)
  leaves the name unset and reports which step failed.
- **Modified marker:** `policy_preset_status` (read) compares each preset
  value with the effective value and answers `{name, modified: [keys]}`.
  The page shows "Balanced (modified: Codex permissions)" the way
  PrusaSlicer shows the orange unlocked lock beside a changed value, and
  each changed control carries a "↶ back to preset" affordance, which is the
  back arrow in PrusaSlicer's tabs. **Reset to preset** is `apply_policy_preset`
  again.
- **Custom** is the name when no preset is applied or the user has touched
  a value and chosen "keep as custom". There is no prompt on leaving a
  preset (PrusaSlicer's Transfer / Discard / Save dialog on preset switch is
  what its users complain about most; the marker and the per-control back
  arrow are enough here, because the whole set is nine values on one page).
- **Compare presets:** a three-column table on the page, current column
  highlighted, in the shape of Codex Permissions' "all three lines, always"
  hint, so the choice is honest about what is not picked. Shipped with
  phase 3, no extra command.
- **User presets** (save the current nine as "Mine"): phase 4, as
  `[policy_presets.<name>]` tables in config.toml, applied by the same
  command. A user preset never inherits from a system one (PrusaSlicer's
  `inherits` and "Detach" exist because its presets have hundreds of keys;
  nine keys do not need a dependency graph).

### 5.4 Interaction with host policy and a paired device

- A preset is host-side by construction: it writes host policy, config.toml
  and user JSON on the host. A window driving a paired host sees the preset
  name and the marker (read, `policy_preset_status` is a read row) and the
  Apply control shows the GrantNote (console-only), like Codex Permissions
  does today.
- The device's own blob is never touched. The mirror keys
  (`codex_permissions`, `pi_project_trust`, …) land in the blob the way
  they do after any accepted set, which on a remote window is the client's
  `remote-ui-state.json`, as today.
- An environment override (`WORKTREES_CODEX_PERMISSIONS`) wins over a
  preset's value for the engine that has it, as it wins over a hand change;
  the marker then shows that key as modified with the source "environment"
  (§6 item 4). The preset must never pretend to have set what the
  environment holds.

### 5.5 Downgrade safety

The nine values are the plain keys an older build already reads. `policy_preset`
is one more key in `host-policy.json`, which "unknown keys survive a write"
already covers (`hostpolicy.rs:85-125`). An older build shows the values with
no preset name; a newer build finds the name again. Nothing is retired from
the blob, so the AGENTS.md downgrade rule holds. A preset's `version` lets a
later release change Balanced's meaning and say so (an offer, fingerprinted
on the version, as `agent-guidance` does) rather than silently.

---

## 6. New options worth adding

Cost: **S** under a day, **M** two to four days, **L** a week or more,
counting the Rust command, the panel, the mock and the check script.

### 6.1 Clearly worth it

| # | Option | What it does | Default | Tier | Page | Why a user wants it | Cost |
|---|---|---|---|---|---|---|---|
| 1 | **Limits panel** | UI for `[limits] max_live_agents`, `max_live.<provider>`, `min_free_memory_pct`, `max_swap_used_pct`, which `guard.rs` already enforces with no UI; shows the RAM-derived default and the live count the way Heavy jobs does | the core's | A | Agent policy | ROADMAP:914-920 asks for it; today the only way to learn the guard exists is a refused launch | M (a `SlotsPanel` twin; a `set_limits` setter in the shape of `set_quota_settings`, admin) |
| 2 | **Policy presets** | §5 | Balanced | B | Agent policy | one word for nine controls on five pages | M+ (the command, the status read, the page, the compare table; L with user presets) |
| 3 | **Confirm the destructive one-clicks** | two-click with a 4 s arm for Remove from Codex / pi, Disconnect kimi, Revoke <repo>, the invalid-edit Reset; a confirm line on Full access ("every Codex lane this computer launches, no prompts, no sandbox"); the 4 s expiry on the three arms that never expire | — | B | Codex, pi, Kimi, Guidance, Agent policy | §2.5 | S (one `useArmed(ms)` hook, used everywhere, replacing five local copies) |
| 4 | **"Set by the environment" indicator** | the policy getters answer `{value, source: "host" \| "env" \| "config" \| "default"}`; a control whose source is `env` is drawn disabled with "Set by WORKTREES_CODEX_PERMISSIONS in this engine's environment" and the variable's value | — | B | Agent policy, Guidance | a control that cannot be moved looks broken; the Kimi page already does this for `WORKTREES_KIMI_MCP` | M (three getters gain a source; the TSX reads it; the mirror rule is unchanged because it already keys on an accepted set) |
| 5 | **Reference panels page** | the seven `files_*` preferences (§1.3) as controls | as today | A | Reference panels | they are preferences with no settings surface; `files_show_ignored` once hid every file a session wrote and the only switch was a toolbar icon | S (keys exist; a page of segmented controls) |
| 6 | **Default model for Claude and Codex in Settings** | the `ModelPicker` the pi and Kimi pages have, for all installed harnesses, on the Agents page; a line saying what the CLI reads instead (`config.toml [model]`) with Reveal | none | B | Agents | today the only place to set Claude's or Codex's default model is the New worktree sheet, and the CLI's `[model]` is a second store nobody is told about. ROADMAP:773-780 ("always pin `--model` at launch, or warn") depends on one answer | S for the picker; M if the CLI store is unified (a comment-preserving `cfg_set`, ROADMAP:2414, which `ai_cmd` editing needs too) |
| 7 | **Reset with a scope** | "Reset to defaults" asks what: Appearance & layout / Window behaviour / Per-place memory (panels, tabs, open files) / Silenced suggestions / Everything; the hint lists what each clears | Appearance & layout | A | Data & privacy | §2.6: today it wipes forty places' memory to reset a theme | S |
| 8 | **Remote-window correctness** | GrantNote on every console/admin section (Guidance, Cross-project, Planning, Profiles, trust, Quota, Slots, Auto-fetch); the Settings-file row shows the client's `remote-ui-state.json`; Restore window hidden remotely | — | — | several | §2.4; every item is a refusal the user meets today by clicking | S |
| 9 | **Notify when an agent waits or finishes while the window is hidden** | a macOS notification (tauri notification plugin) on `waiting`, and on `done` after N minutes hidden; off / waiting only / both | waiting only | B | Agents | ROADMAP:2137 lists "a notification when the window is hidden" as not built; the afterglow dot only works if you are looking | M (plugin + capability + one effect on `sessions:busy`; the remote window must not double-notify) |
| 10 | **Terminal scrollback lines** | expose `TERM_SCROLLBACK` (5000) as a setting, floored at the replay ring's line count so a replay is never truncated | 5000 | A | Terminal | the one xterm knob that costs memory; a user with many tabs wants it lower, a log-reader wants it higher | S (clamp; `termresize-check.mjs` already pins the floor) |

### 6.2 Maybe

| # | Option | Default | Tier | Page | Why / why not | Cost |
|---|---|---|---|---|---|---|
| 11 | Auto-update agent CLIs (Claude, Codex) when a check finds one, and "restart inactive sessions after an update" | off | A | Updates | the restart panel exists (#604); the loop is one toggle away. Against: a CLI update under a running session is the AGENTS.md "zombie server" story | M |
| 12 | Per-harness default view (Chat / Terminal) editable | Chat for Claude/Codex, Terminal for pi/kimi | A | Agents | `agentViewFor` hardcodes it; a kimi user who likes Chat flips every place by hand | S |
| 13 | Chat view: show notices (background tasks, peer messages, compact summaries) inline | on | A | Agents | `turns::Notice` already classifies them; a quiet chat hides them | S |
| 14 | Personal context ceiling ("warn me at 400K") | off | E | Agents | `context-usage.md` Q3; nothing is built underneath | L |
| 15 | `[fanout] launch_gap_secs` / `send_gap_secs` | 5 / 3 | E | Agent policy | ROADMAP:922-933; needs the fan-out itself first | M |
| 16 | `--ai auto`: pick the provider by headroom | off | A | Agents | ROADMAP:935-940; a preset "Autonomous" would want it | M |
| 17 | Usage meter thresholds (amber / red at %) | provider's | E | Layout | the gate has a threshold; the meter's colours do not. ROADMAP:1646 wants to CUT placements first | S |
| 18 | Pull requests poll interval | 60 s | E | General | one more `gh` knob; probably nobody asks | S |
| 19 | Update channel (pre-release) | stable | E | Updates | release.yml would need a pre-release path | M |
| 20 | Global summon hotkey | none | A | Shortcuts | ROADMAP:2363 (needs a capability entry and a key-capture widget) | M |
| 21 | Per-repo "treat as GitHub" override | — | E | Projects | ROADMAP:639-642, `ui-state.json` keyed by repo | S |
| 22 | Sync: auto / scheduled sync, `[sync] hub` surfaced | off | E | Data & privacy | ROADMAP:1941-1945, and the `[sync]` keys have no UI; the hub choice lives in two stores | M |
| 23 | Afterglow on/off as one switch | on | B | Layout | today "off" is a 1 h horizon; a switch is simpler than two sliders, and the sliders go to Expert | S |
| 24 | Launch at login | off | A | General | ordinary for a tray-ish app; tauri autostart plugin | S |
| 25 | Log verbosity / keep N days | — | E | Data & privacy | serve.log rotates already | S |

Not proposed, on purpose: anything in `.worktrees.toml` (ADR 0001 sets the
bar; the project file gets no new keys from this plan), tmux `mouse on`
(ROADMAP:1122, a later layer), per-project PR switch (rejected in
`pull-requests.md` §7), a default-base setting (ROADMAP:2407, replaced by
`origin/HEAD` detection).

---

## 7. Wireframes

Sheet at 1000 × 640 (the modal's size in the screenshots), tokyo-night, the
nav 190 px wide. `●` is a tier dot (B green, A yellow, E red hue in the dot
only; words stay `--txt-hi`/`--txt-dim`). `[chip]` is a scope chip.

### Beginner

```
┌─ Settings ────────────────────────── [Beginner|Advanced|Expert]  esc ⌘, ✕ ─┐
│ WORKSPACE        │ Agent policy                                             │
│   General        │ These settings are this computer's: every window and     │
│ ▸ Appearance     │ every agent it launches.                                 │
│   Layout         │                                                          │
│   Terminal       │ ● Preset                                                 │
│ AGENTS           │   [ Cautious | Balanced ✓ | Autonomous | Custom ]        │
│   Agents         │   Balanced — Codex reviews each command, Claude may not  │
│   Claude         │   branch in (main), agents stop on a nearly spent plan.  │
│   Codex          │   Compare presets ›                                      │
│ AGENT POLICY     │                                                          │
│ ▸ Agent policy   │ ● Codex permissions                                      │
│ PROJECTS         │   [ Ask | Auto-review ✓ | Full access ]                  │
│   Projects       │   Ask: Codex asks before running commands.               │
│ REMOTE           │   Auto-review: a reviewer approves or refuses …          │
│   Remote         │   Full access: no prompts and no sandbox.                │
│ SYSTEM           │                                                          │
│   Updates        │ ● Claude                                                 │
│   Data & privacy │   ☑ Stop Claude from git worktree add and new branches   │
│   Shortcuts      │     in (main)                                            │
│                  │                                                          │
│                  │ ● Plan limits                                            │
│                  │   ☑ Don't start an agent on a nearly spent plan          │
│                  │                                                          │
│                  │ ─ 5 more settings in Advanced ›                          │
└──────────────────┴──────────────────────────────────────────────────────────┘
```

(pi and Kimi pages are absent because their CLIs are not installed on this
host; Reference panels, Profiles and Guidance are above the mode.)

### Advanced

```
┌─ Settings ────────────────────────── [Beginner|Advanced|Expert]  esc ⌘, ✕ ─┐
│ WORKSPACE        │ Terminal                                                 │
│   General        │ These settings are this window's.                        │
│   Appearance     │                                                          │
│   Layout         │ ● Terminal font      [ "SF Mono", Menlo, Monaco …     ]  │
│ ▸ Terminal       │ ● Terminal size      ──────●──────  13px                 │
│   Reference pnls │                                                          │
│ AGENTS           │ ● What a shell tab remembers         [this computer]     │
│   Agents         │   ☑ Keep each tab's output between restarts              │
│   Claude         │   ☑ Give each tab its own command history                │
│   Codex          │   Every window that opens a tab shares its shell.        │
│   pi             │                                                          │
│   Kimi           │ ● Saved terminal history             [this computer]     │
│   Profiles       │   12 tabs, 1.8 MB · ~/Library/…/term-history             │
│   Guidance       │   [ Clear terminal history ]                             │
│ AGENT POLICY     │                                                          │
│   Agent policy   │                                                          │
│ PROJECTS         │                                                          │
│   Projects       │                                                          │
│ REMOTE           │                                                          │
│   Remote         │                                                          │
│ SYSTEM           │                                                          │
│   Updates        │                                                          │
│   Data & privacy │                                                          │
│   Shortcuts      │                                                          │
└──────────────────┴──────────────────────────────────────────────────────────┘
```

### Expert, on a window driving a paired host

```
┌─ Settings ─────────────────────── [Beginner|Advanced|Expert]  esc ⌘, ✕ ─┐
│ WORKSPACE        │ Remote                                                │
│   General        │                                                       │
│   Appearance     │ ● This window drives                 [this window]   │
│   Layout         │   ○ This machine     ● studio-mac  192.168.4.7 ·     │
│   Terminal       │     paired 2026-10-29  [Forget] [Paste a new address]│
│   Reference pnls │   Pair with a host   [ worktrees-pair:v1… ] [Pair]   │
│ AGENTS           │                                                       │
│   Agents         │ ● Let devices reach this computer    [studio-mac]    │
│   …              │   ⓘ Set up at studio-mac itself; a paired device may  │
│ AGENT POLICY     │     read it. Listening on 192.168.4.7:51820 (LAN).    │
│   Agent policy   │                                                       │
│ PROJECTS         │ ● Paired devices                     [studio-mac]  2 │
│   Projects       │   ⓘ Set up at studio-mac itself.                      │
│ REMOTE           │   iPhone · Operate · every project · step-up on       │
│ ▸ Remote      2  │   this window · Drive · 3 projects · last seen now    │
│ SYSTEM           │                                                       │
│   Updates        │ ● Announce on the network            [studio-mac]    │
│   Data & privacy │   ⓘ Set up at studio-mac itself.  on                  │
│   Shortcuts      │                                                       │
└──────────────────┴───────────────────────────────────────────────────────┘
```

The Beginner wireframe's "5 more settings in Advanced ›" is the only
affordance the mode needs beyond the switch. The chip after a section title
replaces the per-control "Kept by this machine rather than this window"
sentences.

---

## 8. Phased implementation plan

| Phase | Ships | Needs a migration? | Size | Risk |
|---|---|---|---|---|
| **0. Regroup** | `CATS` gains `group`; the moves in §3.3; `LEGACY_CATS` + `FOCUS_HOME` with a drift check; page scope sentences and chips; `data-focus` on Codex Permissions; GrantNotes everywhere (item 8); confirm arms (item 3); Reset scope (item 7) | **No.** No blob key changes. `offers-check.mjs`, `hostpolicy-check.mjs`, `layout-check.mjs` and `quota-check.mjs` keep passing because no key or command moves; `quota::SETTINGS_PATH` names "Settings → Behavior → Plan limits" in a refusal message and must be re-pointed (its check script will catch it) | M (3–4 days) | the mock drives all of it; the remote window needs a real paired host for the GrantNote pass (`engine-client-manual-checks.md`). `SettingsSheet.tsx` is 1 844 lines and most of the move is JSX relocation; split the pages into files while moving (`pages/*.tsx`), which the Explore read found the sheet already half does |
| **1. Modes** | `settings_mode` key, rev 5 (`expert` for an existing blob, `beginner` fresh); tier per section (a `tier` prop on `<Section>`), rule 2's chip via a `nonDefault(section)` predicate over the keys the section owns, rule 3/4; the footer line; deep-link reveal | **One new key**, additive; a downgrade ignores it | M (3 days) | the "changed setting is never hidden" predicate needs each section to DECLARE its keys (the same declaration `FOCUS_HOME` and the Reset scope need), and a check script that every blob key and policy getter is claimed by exactly one section, or a key drifts into "never shown" silently |
| **2. New options A** | items 1, 5, 6 (picker half), 9, 10, 12, 13, 23 | `[limits]` setter (config.toml, admin); the notification plugin's capability entry | L (1–2 weeks across lanes) | item 9 on a remote window; item 1's setter must refuse what `guard.rs` would ignore and list problems like Quota does |
| **3. Presets** | `policy_presets.rs`, `apply_policy_preset` (console), `policy_preset_status` (read), the Preset section, Compare, item 4's `source` on the getters | **One host-policy key** (`policy_preset`), additive | L (1 week) | ordering of the setters under one audit episode; an env override mid-apply; the mirror writes after each accepted set stay as they are |
| **4. Later** | user presets (`[policy_presets.*]`), the Projects table's profile/private columns writing through the existing setters, item 6's CLI unification (`cfg_set`), items 11, 16, 22 | `cfg_set` must preserve comments in config.toml (ROADMAP:2414) | L | `~/.config/worktrees/config.toml` is edited by hand by the same user; a writer that reorders or strips comments is a regression the quota panel avoided by writing only its own table |

Phase 0 alone removes most of §2. Phases 1 and 3 are the owner's two
questions and are independent of each other; either can be skipped without
unpicking the other.

What every phase keeps: the whole-blob save, the mirror keys and their rule,
the five host-policy setters, the offers' precondition rule, `useEscape` for
any new surface (none is planned: the mode switch is a segmented control),
the contrast rule (tier dots carry hue, words do not), and ADR 0001 (no key
is added to `.worktrees.toml`).

---

## 9. Open questions for the owner

1. **Fresh-install mode: Beginner or Advanced?** §4.7 recommends Beginner
   with the footer line; the counter-argument is that nobody installing this
   app is a beginner at the domain.
2. **Should Codex Permissions and the (main) guard be Beginner-tier (this
   plan) or Advanced?** Beginner shows them because a new user should see
   what Auto-review means before the first lane runs; Advanced would make the
   Beginner policy page just the preset.
3. **Presets: should `ai_auto_resume` be in them after all?** The brief
   named it; §5.2 leaves it out because it is window-scoped. If it goes in,
   the preset gains a "window half" applied to the window that pressed Apply,
   and the page must say so.
4. **Does "Autonomous" set Codex to Full access, or stop at Auto-review?**
   The table says Full; the confirm on Full access (item 3) still fires on
   Apply.
5. **The harness pages: fold the four into one "Harnesses" page with tabs,
   or keep four nav rows (this plan)?** Four rows keep deep links trivial
   and each page short; one page shortens the nav by three in Expert.
6. **Where does the per-project table belong: Settings → Projects (this
   plan) or only the Project sheet?** `project-settings.md` §10 chose the
   sheet for editing; the table here is a list with links, not a second
   editor, but it is one more place a project's planning level appears.
7. **Should a paired device be able to change the default agent and model
   (shared, via the unbuilt `patch_settings`), or keep them per device as
   today?** The reorg does not depend on the answer, but the Agents page's
   scope sentence does.
8. **Item 9 (notifications): in scope for this reorg, or its own lane?** It
   is the one "clearly worth it" option with a plugin dependency.
9. **Usage: keep it (Expert, collapsed under Data & privacy) or cut it as
   ROADMAP:1685 planned?** The reorg's answer is "keep, hidden"; cutting it
   is one fewer page to carry.

---

## Appendix A. Sources

- Code: `app/src/SettingsSheet.tsx`, `settings.ts`, `offers.ts`, `ipc.ts`
  (the remote door), every `*Panel.tsx`, `HostPolicyControls.tsx`,
  `AgentSetup.tsx`, `ProjectSheet.tsx`, `StatusSheet.tsx`;
  `crates/worktrees-api/src/{hostpolicy,prefs,project,ops,config}.rs`;
  `crates/worktrees-core/src/{projcfg,config,quota,slots,guard,reach,
  planning,guidance,profile,trust,kimiplace,codexmcp,pimcp,mcpsetup}.rs`;
  `crates/worktrees-cli/src/serve/dispatch.rs` (the capability of every row).
- Docs: `docs/proposals/*` (project-settings §5, §10; server-edition App. B;
  server-edition-phase2 §2.2, §3.4, §4.4, §5.3, §9.1; owned-planning §2.2;
  agent-guidance Q8; cross-project §5; chat-session-view §1, §9;
  harness-restart; pull-requests §7), `docs/adr/0001`, ROADMAP.md (lines
  cited inline), DESIGN.md.
- Screenshots of every category on the mock harness (port 1457, 1440 × 900):
  `~/.cache/worktrees/worktrees/settings-reorg-plan/settings-NN-<cat>.png`,
  with `labels.json` listing every label, sub-label, checkbox and button the
  DOM showed per category.
- PrusaSlicer: [Simple, Advanced, Expert modes](https://help.prusa3d.com/en/article/simple-advanced-expert-modes_1765),
  [UI overview](https://help.prusa3d.com/en/article/ui-overview_1766) (the
  mode switch's position), [Miscellaneous](https://help.prusa3d.com/en/article/miscellaneous_1755)
  (the lock icon, the back arrow, "Compare preset with another", saving
  under User presets), [Preferences](https://help.prusa3d.com/en/article/preferences_124601)
  (the unsaved-changes prompt, Transfer, "Remember my choice", "Update
  built-in presets automatically", "PrusaSlicer NEVER deletes user-made
  profiles"), a [2.3.3 source mirror](https://grok.dragonflybsd.org/source/xref/dports/cad/PrusaSlicer/PrusaSlicer-version_2.3.3/src/libslic3r/PrintConfig.cpp)
  for the `comSimple` / `comAdvanced` / `comExpert` tags (the mirror
  answered 404 at fetch time; the tag names come from search snippets of
  it and of `Config.hpp`, not from a fetched page). The articles are filed
  under "PrusaSlicer 2.9 (Legacy)". "Detach from system preset" and the
  `inherits` key were not found in official pages, only in forum answers,
  and are not relied on above.
