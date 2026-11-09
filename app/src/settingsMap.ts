// The Settings sheet's map (docs/proposals/settings-reorg.md §3–§4): the six
// groups, the pages, every section with its tier, its scope and the values it
// owns, the old category ids that must keep resolving, and the mode rules.
//
// PURE and import-free at runtime (type imports only, which esbuild erases):
// `settingsmap-check.mjs` loads this file as a data: module, the way
// zoom-check loads settings.ts, so it can hold the rules to the JSX and to the
// blob without a second copy. Nothing here reads the DOM, the host or React.
//
// The one invariant the whole map serves — §4.2 rule 1, HIDDEN IS NEVER OFF: a
// mode decides what the sheet DRAWS and nothing else. `settings_mode` is read
// here and by SettingsSheet, never by anything that acts on a setting
// (`settingsmap-check.mjs` greps for it).
import type { Settings } from "./settings";
import type { Harness } from "./harness";

// ── modes ───────────────────────────────────────────────────────────────────

export const MODES = ["beginner", "advanced", "expert"] as const;
export type Mode = (typeof MODES)[number];
export const MODE_LABEL: Record<Mode, string> = { beginner: "Beginner", advanced: "Advanced", expert: "Expert" };
const RANK: Record<Mode, number> = { beginner: 0, advanced: 1, expert: 2 };
export const isMode = (v: unknown): v is Mode => typeof v === "string" && (MODES as readonly string[]).includes(v);

// ── groups and pages ────────────────────────────────────────────────────────

export const GROUPS = [
  { id: "workspace", label: "Workspace" },
  { id: "agents", label: "Agents" },
  { id: "policy", label: "Agent policy" },
  { id: "projects", label: "Projects" },
  { id: "remote", label: "Remote access" },
  { id: "system", label: "System" },
] as const;
export type GroupId = (typeof GROUPS)[number]["id"];

/** Whose a page's settings are, said once under its title (§3.1) instead of
 *  per control. `null`: the page mixes homes, and each section carries its own
 *  chip (Remote, Updates, Data). */
export type Scope = "window" | "computer";

export const PAGES = [
  { id: "general", label: "General", group: "workspace", scope: "window" },
  { id: "appearance", label: "Appearance", group: "workspace", scope: "window" },
  { id: "layout", label: "Layout", group: "workspace", scope: "window" },
  { id: "terminal", label: "Terminal", group: "workspace", scope: "window" },
  { id: "reference", label: "Reference panels", group: "workspace", scope: "window" },
  { id: "agents", label: "Agents", group: "agents", scope: "window" },
  { id: "claude", label: "Claude", group: "agents", scope: "computer" },
  { id: "codex", label: "Codex", group: "agents", scope: "computer" },
  { id: "pi", label: "pi", group: "agents", scope: "computer" },
  { id: "kimi", label: "Kimi", group: "agents", scope: "computer" },
  { id: "profiles", label: "Profiles", group: "agents", scope: "computer" },
  { id: "guidance", label: "Guidance", group: "agents", scope: "computer" },
  { id: "policy", label: "Agent policy", group: "policy", scope: "computer" },
  { id: "projects", label: "Projects", group: "projects", scope: "computer" },
  { id: "remote", label: "Remote", group: "remote", scope: null },
  { id: "updates", label: "Updates", group: "system", scope: null },
  { id: "data", label: "Data & privacy", group: "system", scope: null },
  { id: "shortcuts", label: "Shortcuts", group: "system", scope: null },
] as const satisfies readonly { id: string; label: string; group: GroupId; scope: Scope | null }[];
export type PageId = (typeof PAGES)[number]["id"];
export const PAGE: Record<PageId, (typeof PAGES)[number]> = Object.fromEntries(PAGES.map((p) => [p.id, p])) as never;
export const isPage = (v: unknown): v is PageId => typeof v === "string" && PAGES.some((p) => p.id === v);

/** The sentences a page's scope is said in (§3.1). On a window driving a
 *  paired host, "this computer" is that host, by name. */
export function scopeSentence(scope: Scope | null, host: string | null): string | null {
  if (scope === "window") return "These settings are this window's.";
  if (scope === "computer") {
    return host
      ? `These settings are ${host}'s: every window and every agent it launches. Some are set up at ${host} itself.`
      : "These settings are this computer's: every window and every agent it launches.";
  }
  return null;
}
/** A section's chip when its home differs from its page's. */
export const scopeChip = (scope: Scope, host: string | null) => (scope === "window" ? "this window" : host ?? "this computer");

// ── the values a section may own ────────────────────────────────────────────

/** A host-side value a section owns, as "differs from the shipped default"
 *  (§4.2 rule 2). The sheet fills these in from the getters when it opens
 *  (`SettingsSheet`'s host snapshot); absent means "not known", which never
 *  shows a section on its own. */
export const HOST_KEYS = [
  "codex_access", // get_codex_permissions ≠ auto-review
  "main_guard", // agent_guidance_status .settings.guard on
  "guidance_enabled", // agent_guidance_status .settings.enabled off
  "guidance_skill", // agent_guidance_status .skill_edit present
  "pi_trust", // pi_status .trust ≠ never
  "pi_allowed", // pi_status .allowed non-empty
  "kimi_allowed", // kimi_status .allowed non-empty
  "cross_project", // cross_project_status .level ≠ off
  "private_projects", // cross_project_status, any project private
  "quota_gate", // quota_settings .gate off
  "quota_pct", // quota_settings .weekly_warn_pct set
  "slots", // slots_settings, any [slots] entry
  "limits", // limits_settings, any [limits] entry
  "policy_preset", // policy_preset_status, a preset other than Balanced, or modified
  "fetch_interval", // get_fetch_interval ≠ 0
  "term_history", // get_term_history_opts, either switch off
  "planning_default", // planning_status .default set
  "profiles", // profiles_info, any profile
  "remote_access", // remote_status on
  "devices", // devices_list non-empty
] as const;
export type HostKey = (typeof HOST_KEYS)[number];

/** The host getters every HostKey comes from. `settingsmap-check.mjs` holds
 *  this against the host-policy getters in worktrees-api: a getter no section
 *  claims is a value that could be changed and never shown. */
export const HOST_GETTERS: Record<HostKey, string> = {
  codex_access: "get_codex_permissions",
  main_guard: "agent_guidance_status",
  guidance_enabled: "agent_guidance_status",
  guidance_skill: "agent_guidance_status",
  pi_trust: "pi_status",
  pi_allowed: "pi_status",
  kimi_allowed: "kimi_status",
  cross_project: "cross_project_status",
  private_projects: "cross_project_status",
  quota_gate: "quota_settings",
  quota_pct: "quota_settings",
  slots: "slots_settings",
  limits: "limits_settings",
  policy_preset: "policy_preset_status",
  fetch_interval: "get_fetch_interval",
  term_history: "get_term_history_opts",
  planning_default: "planning_status",
  profiles: "profiles_info",
  remote_access: "remote_status",
  devices: "devices_list",
};

/** Blob keys no section owns, each for a stated reason. Every key `Settings`
 *  has is in exactly one of: a section's `keys`, or one of these four lists
 *  (`settingsmap-check.mjs`). */
export const MEMORY_KEYS = [
  // the dock and the nav remembering what you did with them (§1.3)
  "dock_open", "dock_width", "dock_tab", "dock_lower_frac", "files_diff",
  "sort_mode", "sort_dir", "sync_with_sessions",
  // which nav view is showing: All projects or Workspaces (workspaces.ts)
  "nav_view",
] as const;
/** Memory kept PER PLACE (or per project): what Reset's "Per-place memory"
 *  clears, and what no other scope may touch. */
export const PLACE_MEMORY_KEYS = [
  "place_panels", "term_tab_names", "term_tabs", "term_tab_active", "docs_collapsed",
  "files_open", "files_open_at", "collapsed", "manual_order",
  // a named workspace's folding, and its projects' (workspaces.ts)
  "ws_collapsed",
] as const;
/** Markers the controls write, never a control of their own. */
export const MARKER_KEYS = ["settings_rev", "last_seen_version", "update_dismissed", "chat_default_v1", "init_dismissed", "offers_dismissed"] as const;
/** An older build's copy of host policy (settings.ts, end of the type). */
export const MIRROR_KEYS = ["codex_permissions", "pi_project_trust", "fetch_interval_min", "term_persist_scrollback", "term_per_tab_history"] as const;
/** The sheet's own view: the mode switch in its header. */
export const SHEET_KEYS = ["settings_mode"] as const;

// ── sections ────────────────────────────────────────────────────────────────

export type SectionDef = {
  /** Also its `data-focus`, so a deep link can name it. */
  id: string;
  page: PageId;
  tier: Mode;
  label: string;
  /** The person's words for it, for the header's search (settingsSearch.ts):
   *  what someone types who does not know the sheet's label ("dark mode" →
   *  Theme, "cap agents" → Limits). Hand-written and lowercase; never the
   *  text of the controls inside, which the owner chose not to index. Every
   *  section has some (`settingsmap-check.mjs` rule 9). */
  keywords: readonly string[];
  /** The blob keys it shows a control for. */
  keys?: readonly (keyof Settings)[];
  /** The host values it shows a control for. */
  host?: readonly HostKey[];
  /** Its home, when that is not its page's (the chip, §3.1). */
  scope?: Scope;
  /** A harness page's own section: Beginner while that CLI is installed on
   *  the host (rule 4), Advanced otherwise. */
  harness?: Harness;
  /** serve's row the section READS: a remote window that may not ask for it
   *  shows the GrantNote in its place. */
  read?: string;
  /** serve's row the section WRITES: a remote window that may not ask for it
   *  shows the value with its controls disabled, and says why (§6 item 8). */
  write?: string;
  /** Drawn only on the window's own host (`restore_window` is read by the
   *  host's shim at launch; on a remote window nothing reads it). */
  localOnly?: boolean;
  /** Only meaningful while this holds (the light/dark pair, for System). */
  applies?: (s: Settings) => boolean;
};

export const SECTIONS: readonly SectionDef[] = [
  // ── General
  { id: "restore-last", page: "general", tier: "beginner", label: "Startup", keywords: ["reopen", "restore", "last place", "on launch", "open at start"], keys: ["restore_last"] },
  { id: "restore-window", page: "general", tier: "advanced", label: "Restore window", keywords: ["window size", "window position", "remember window", "reopen window"], keys: ["restore_window"], localOnly: true },
  { id: "editor", page: "general", tier: "beginner", label: "Editor", keywords: ["code editor", "ide", "vscode", "vs code", "cursor", "zed", "open in editor"], keys: ["editor_cmd"] },
  { id: "terminal-command", page: "general", tier: "advanced", label: "Terminal command", keywords: ["terminal app", "external terminal", "open in terminal", "iterm", "ghostty"], keys: ["terminal_cmd"] },
  { id: "pull-requests", page: "general", tier: "beginner", label: "Pull requests", keywords: ["prs", "pr status", "github", "gh"], keys: ["pull_requests"] },
  { id: "auto-fetch", page: "general", tier: "advanced", label: "Git", keywords: ["fetch", "git fetch", "background fetch", "refresh remotes", "interval"], host: ["fetch_interval"], scope: "computer", write: "set_fetch_interval" },
  { id: "projects-parent", page: "general", tier: "advanced", label: "New projects", keywords: ["clone", "clone folder", "project folder", "new project location", "directory"], keys: ["projects_parent"] },
  // One row per beta feature (beta.ts). Advanced and Expert only (the owner's
  // call, 2026-11-08); a feature turned on is a changed value, so rule 2
  // shows the section in Beginner too, where it can be turned off again.
  { id: "beta", page: "general", tier: "advanced", label: "Beta features", keywords: ["beta", "experimental", "preview", "early access", "feature flag", "ideas", "workspaces"], keys: ["beta_ideas"], localOnly: true },
  // ── Appearance
  { id: "zoom", page: "appearance", tier: "beginner", label: "Overall size", keywords: ["zoom", "scale", "bigger", "smaller", "text size", "magnify"], keys: ["app_zoom"] },
  { id: "ui-font", page: "appearance", tier: "advanced", label: "UI font size", keywords: ["font", "font size", "text size", "interface font", "rem"], keys: ["ui_rem"] },
  { id: "theme", page: "appearance", tier: "beginner", label: "Theme", keywords: ["dark mode", "light mode", "colors", "colours", "color scheme", "night mode"], keys: ["theme"] },
  { id: "theme-pair", page: "appearance", tier: "advanced", label: "Light ↔ dark pair", keywords: ["dark mode", "light mode", "follow system", "system theme", "auto theme"], keys: ["theme_light", "theme_dark"], applies: (s) => s.theme === "system" },
  { id: "density", page: "appearance", tier: "advanced", label: "Density", keywords: ["compact", "spacing", "padding", "dense", "row height"], keys: ["density"] },
  // ── Layout
  { id: "sidebar", page: "layout", tier: "beginner", label: "Sidebar", keywords: ["nav", "navigation", "places list", "pin sidebar", "hide sidebar", "hover reveal"], keys: ["nav_pinned", "nav_hover_reveal"] },
  { id: "layout", page: "layout", tier: "beginner", label: "Layout", keywords: ["swap sides", "dock side", "places side", "move panels", "left", "right"], keys: ["places_side", "dock_side"] },
  { id: "nav-width", page: "layout", tier: "advanced", label: "Nav width", keywords: ["sidebar width", "places width", "wider sidebar", "nav size"], keys: ["nav_width"] },
  { id: "nav-tiers", page: "layout", tier: "advanced", label: "Nav tiers", keywords: ["hide idle places", "hide dormant places", "nav groups", "sidebar sections"], keys: ["hidden_tiers"] },
  { id: "dock-lower", page: "layout", tier: "advanced", label: "Terminal placement", keywords: ["terminal position", "terminal below", "split terminal", "reference column"], keys: ["dock_lower"] },
  { id: "usage-meter", page: "layout", tier: "advanced", label: "Usage meter", keywords: ["usage", "quota meter", "plan meter", "rate limit", "status bar"], keys: ["usage_place"] },
  { id: "tree-guides", page: "layout", tier: "expert", label: "Tree guide lines", keywords: ["indent guides", "tree lines", "nav lines", "outline"], keys: ["nav_guides"] },
  { id: "afterglow", page: "layout", tier: "expert", label: "Afterglow", keywords: ["done ring", "finished dot", "fade", "unread", "decay"], keys: ["done_horizon_secs", "done_steps"] },
  // ── Terminal
  { id: "term-font", page: "terminal", tier: "beginner", label: "Terminal font", keywords: ["font", "font size", "font family", "monospace", "typeface", "terminal text size"], keys: ["term_family", "term_size"] },
  { id: "term-scrollback", page: "terminal", tier: "advanced", label: "Scrollback", keywords: ["history lines", "buffer", "lines kept", "scroll history"], keys: ["term_scrollback"] },
  { id: "term-remembers", page: "terminal", tier: "advanced", label: "What a shell tab remembers", keywords: ["shell history", "per tab history", "persist scrollback", "restore terminal", "zsh history"], host: ["term_history"], scope: "computer", write: "set_term_history_opts" },
  { id: "term-history", page: "terminal", tier: "advanced", label: "Saved terminal history", keywords: ["clear history", "saved scrollback", "delete history", "terminal data"], scope: "computer", read: "term_history_info", write: "term_history_clear" },
  // ── Reference panels
  { id: "files-layout", page: "reference", tier: "advanced", label: "Files layout", keywords: ["file tree", "side by side", "stacked", "split", "reader layout"], keys: ["files_layout", "files_split_pct", "files_stack_pct"] },
  { id: "files-reading", page: "reference", tier: "advanced", label: "Reading", keywords: ["markdown", "line wrap", "word wrap", "source view", "markdown zoom", "svg"], keys: ["files_wrap", "files_md_source", "files_md_zoom"] },
  { id: "files-tree", page: "reference", tier: "advanced", label: "File tree", keywords: ["ignored files", "gitignore", "changed files", "hidden files"], keys: ["files_show_ignored", "files_changed_only"] },
  { id: "files-diff", page: "reference", tier: "advanced", label: "Diffs", keywords: ["diff base", "compare", "merge base", "changes against"], keys: ["files_diff_base"] },
  // ── Agents
  { id: "default-agent", page: "agents", tier: "beginner", label: "Default agent", keywords: ["provider", "harness", "which ai", "claude", "codex", "pi", "kimi"], keys: ["default_provider"] },
  { id: "default-models", page: "agents", tier: "beginner", label: "Default model", keywords: ["models", "ai model", "llm", "opus", "sonnet", "gpt"], keys: ["default_models"] },
  { id: "auto-resume", page: "agents", tier: "beginner", label: "Resume on open", keywords: ["continue conversation", "last conversation", "auto resume", "reattach"], keys: ["ai_auto_resume"] },
  { id: "chat-view", page: "agents", tier: "beginner", label: "Chat view", keywords: ["chat", "transcript", "conversation view", "messages"], keys: ["chat_view"] },
  { id: "notifications", page: "agents", tier: "beginner", label: "Notifications", keywords: ["notify", "alerts", "banner", "when done", "notification center"], keys: ["notify_hidden"] },
  { id: "connections", page: "agents", tier: "beginner", label: "Worktrees tools for agents", keywords: ["mcp", "mcp server", "tools", "connect agents", "integrations"], scope: "computer" },
  { id: "plan-usage", page: "agents", tier: "advanced", label: "Plan usage", keywords: ["usage", "quota", "rate limit", "subscription", "account usage"], keys: ["usage_claude", "usage_codex", "usage_kimi"] },
  { id: "ai-command", page: "agents", tier: "expert", label: "AI command (CLI)", keywords: ["cli", "ai flag", "default command", "launch command"], scope: "computer" },
  // ── Claude
  { id: "mcp-server", page: "claude", tier: "beginner", label: "Claude MCP server", keywords: ["mcp", "claude tools", "install mcp", "integration"], harness: "claude", read: "mcp_status", write: "mcp_install" },
  { id: "claude-policy", page: "claude", tier: "beginner", label: "Agent policy", keywords: ["claude permissions", "safety", "autonomy"], harness: "claude" },
  // ── Codex
  { id: "codex-skills", page: "codex", tier: "advanced", label: "Your skills", keywords: ["skills", "link skills", "claude skills", "agents skills"], write: "agent_link_skills" },
  { id: "codex-mcp", page: "codex", tier: "beginner", label: "Codex MCP server", keywords: ["mcp", "codex tools", "install mcp", "integration"], harness: "codex", read: "codex_mcp_status", write: "codex_mcp_install" },
  { id: "codex-policy", page: "codex", tier: "beginner", label: "Agent policy", keywords: ["codex permissions", "approvals", "safety", "autonomy"], harness: "codex" },
  // ── pi
  { id: "pi-mcp", page: "pi", tier: "beginner", label: "pi MCP server", keywords: ["mcp", "pi tools", "install mcp", "integration"], harness: "pi", read: "pi_mcp_status", write: "pi_mcp_install" },
  { id: "pi-install", page: "pi", tier: "advanced", label: "pi", keywords: ["install pi", "pi cli", "setup pi", "preflight"], harness: "pi" },
  { id: "pi-models", page: "pi", tier: "beginner", label: "Models", keywords: ["pi models", "model list", "providers", "allowed models"], harness: "pi" },
  { id: "pi-policy", page: "pi", tier: "beginner", label: "Agent policy", keywords: ["pi permissions", "pi trust", "safety", "autonomy"], harness: "pi" },
  // ── Kimi
  { id: "kimi-mcp", page: "kimi", tier: "beginner", label: "kimi MCP server", keywords: ["mcp", "kimi tools", "install mcp", "integration"], harness: "kimi", read: "kimi_mcp_status", write: "kimi_mcp_set" },
  { id: "kimi-install", page: "kimi", tier: "advanced", label: "kimi", keywords: ["install kimi", "kimi cli", "setup kimi"], harness: "kimi" },
  { id: "kimi-models", page: "kimi", tier: "beginner", label: "Models", keywords: ["kimi models", "model list", "providers"], harness: "kimi" },
  { id: "kimi-policy", page: "kimi", tier: "beginner", label: "Agent policy", keywords: ["kimi permissions", "kimi trust", "safety", "autonomy"], harness: "kimi" },
  // ── Profiles
  { id: "profiles", page: "profiles", tier: "advanced", label: "AI profiles", keywords: ["profile", "accounts", "switch account", "config dir", "claude config"], host: ["profiles"], read: "profiles_info", write: "save_profile" },
  // ── Guidance
  { id: "agent-guidance", page: "guidance", tier: "advanced", label: "Agent guidance", keywords: ["guidance", "instructions", "launch guidance", "agents md", "rules"], host: ["guidance_enabled"], write: "set_agent_guidance" },
  { id: "guidance-policy", page: "guidance", tier: "advanced", label: "Agent policy", keywords: ["guard", "main guard", "safety", "rules"] },
  { id: "guidance-skill", page: "guidance", tier: "expert", label: "The skill", keywords: ["edit skill", "skill text", "custom guidance", "instructions", "prompt"], host: ["guidance_skill"], write: "set_agent_guidance_skill" },
  // ── Agent policy
  { id: "policy-preset", page: "policy", tier: "beginner", label: "Preset", keywords: ["presets", "safety", "autonomy", "risk level"], host: ["policy_preset"], write: "apply_policy_preset" },
  { id: "codex-permissions", page: "policy", tier: "beginner", label: "Codex permissions", keywords: ["permissions", "codex access", "approvals", "sandbox", "auto review", "full access"], host: ["codex_access"], write: "set_codex_permissions" },
  { id: "main-guard", page: "policy", tier: "beginner", label: "Claude in (main)", keywords: ["guard", "protect main", "main branch", "git worktree add", "branching"], host: ["main_guard"], write: "set_agent_guidance" },
  { id: "plan-limits", page: "policy", tier: "beginner", label: "Plan limits", keywords: ["quota", "usage limit", "spent plan", "rate limit", "gate"], host: ["quota_gate"], write: "set_quota_settings" },
  { id: "plan-threshold", page: "policy", tier: "advanced", label: "Weekly threshold", keywords: ["weekly limit", "quota threshold", "percent", "warn"], host: ["quota_pct"], write: "set_quota_settings" },
  { id: "pi-trust", page: "policy", tier: "advanced", label: "pi", keywords: ["pi permissions", "pi trust", "project trust", "allowed repos"], host: ["pi_trust", "pi_allowed"], write: "set_pi_trust" },
  { id: "kimi-trust", page: "policy", tier: "advanced", label: "Kimi", keywords: ["kimi permissions", "kimi trust", "folder trust", "allowed folders"], host: ["kimi_allowed"], write: "set_kimi_allowed" },
  { id: "cross-project", page: "policy", tier: "advanced", label: "Other projects", keywords: ["cross project", "other repos", "read other projects", "private projects", "reach", "access"], host: ["cross_project", "private_projects"], write: "set_cross_project" },
  { id: "limits", page: "policy", tier: "advanced", label: "Limits", keywords: ["max agents", "cap agents", "agent limit", "how many agents", "live agents", "concurrency", "memory", "swap"], host: ["limits"], write: "set_limits" },
  { id: "heavy-jobs", page: "policy", tier: "advanced", label: "Heavy jobs", keywords: ["slots", "build slots", "parallel builds", "concurrency", "gates"], host: ["slots"], write: "set_slot_capacity" },
  // ── Projects
  { id: "planning", page: "projects", tier: "beginner", label: "Planning", keywords: ["plan files", "planning with files", "task plan", "planning profile"], host: ["planning_default"], write: "set_planning_default" },
  // ── Remote
  { id: "this-window-drives", page: "remote", tier: "beginner", label: "This window drives", keywords: ["remote host", "drive host", "which machine", "paired host", "other computer", "connect to host"], scope: "window" },
  { id: "remote-access", page: "remote", tier: "advanced", label: "Let devices reach this computer", keywords: ["remote", "lan", "network", "phone", "iphone", "pairing", "kill switch"], host: ["remote_access"], scope: "computer" },
  { id: "devices", page: "remote", tier: "advanced", label: "Paired devices", keywords: ["phone", "iphone", "ipad", "pair", "pairing", "revoke"], host: ["devices"], scope: "computer" },
  // ── Updates
  { id: "version", page: "updates", tier: "beginner", label: "Version", keywords: ["update", "upgrade", "check for updates", "auto update", "release", "about"], keys: ["update_auto_check"] },
  { id: "agent-clis", page: "updates", tier: "advanced", label: "Agent CLIs", keywords: ["update claude", "update codex", "upgrade", "cli versions", "harness updates"], scope: "computer", write: "update_harness" },
  { id: "install-version", page: "updates", tier: "expert", label: "Install another version", keywords: ["downgrade", "rollback", "older version", "specific version"] },
  // ── Data & privacy
  { id: "feedback", page: "data", tier: "beginner", label: "Send feedback", keywords: ["bug report", "report a bug", "contact", "support"] },
  { id: "logs", page: "data", tier: "advanced", label: "Logs", keywords: ["log", "debug", "diagnostics", "errors", "serve log", "app log"], scope: "computer", write: "log_tail" },
  { id: "settings-file", page: "data", tier: "advanced", label: "Settings file", keywords: ["ui state", "json", "config file", "reveal", "settings json"], scope: "window" },
  { id: "reset", page: "data", tier: "advanced", label: "Reset", keywords: ["defaults", "restore defaults", "factory reset", "clear settings", "start over"], scope: "window" },
  { id: "usage", page: "data", tier: "expert", label: "Usage record", keywords: ["analytics", "telemetry", "events", "privacy", "tracking"], scope: "computer", write: "ui_events_clear" },
  // ── Shortcuts
  { id: "shortcuts", page: "shortcuts", tier: "beginner", label: "Shortcuts", keywords: ["keyboard", "keys", "hotkeys", "keybindings", "chords"] },
];
export const SECTION: Record<string, SectionDef> = Object.fromEntries(SECTIONS.map((s) => [s.id, s]));

/** Deep-link targets INSIDE a section: the focus id names a band of it, and
 *  revealing it means revealing the section that holds it. */
export const SUB_FOCUS: Record<string, string> = {
  "agent-guidance-changed": "guidance-skill",
};

/** Which page each focus id lives on, sheet-wide (§3.4): a caller that names a
 *  focus is sent to wherever that section is now, whatever page it named. */
export const FOCUS_HOME: Record<string, PageId> = {
  ...Object.fromEntries(SECTIONS.map((s) => [s.id, s.page])),
  ...Object.fromEntries(Object.entries(SUB_FOCUS).map(([f, s]) => [f, SECTION[s].page])),
};

/** A category that no longer exists, mapped to where its sections went
 *  (§3.4). FOREVER: an offer, an older client, a remote store opener or a
 *  release note may still name one. A `focus` the caller gave wins; the alias
 *  supplies one for callers that named only the category. `remote` is both an
 *  old id (Remote access) and a page: opened by its old name it lands on the
 *  listener's section. */
export const LEGACY_CATS = {
  navigation: { cat: "layout" },
  commands: { cat: "agents" },
  ai: { cat: "profiles" },
  planning: { cat: "projects", focus: "planning" },
  behavior: { cat: "general" },
  hosts: { cat: "remote", focus: "this-window-drives" },
  remote: { cat: "remote", focus: "remote-access" },
  devices: { cat: "remote", focus: "devices" },
  usage: { cat: "data", focus: "usage" },
} as const satisfies Record<string, { cat: PageId; focus?: string }>;
export type LegacyCat = keyof typeof LEGACY_CATS;
/** What a caller may ask the sheet to open at. */
export type CatId = PageId | LegacyCat;
export type SettingsAt = { cat: CatId; focus?: string };

/** Where `at` lands: the page, and the focus (a section id or a band in one). */
export function resolveAt(at: SettingsAt): { cat: PageId; focus?: string } {
  const alias = (LEGACY_CATS as Record<string, { cat: PageId; focus?: string }>)[at.cat];
  let cat: PageId = alias ? alias.cat : isPage(at.cat) ? at.cat : "general";
  const focus = at.focus ?? alias?.focus;
  if (focus && FOCUS_HOME[focus]) cat = FOCUS_HOME[focus];
  return focus ? { cat, focus } : { cat };
}
/** The section a focus id reveals. */
export const focusSection = (focus: string): string | null => SUB_FOCUS[focus] ?? (SECTION[focus] ? focus : null);

// ── the mode rules (§4.2) ───────────────────────────────────────────────────

/** Why a section is drawn, or null when the mode hides it:
 *  - `mode`: its tier is at or below the mode;
 *  - `problem`: rule 3, something is wrong in it;
 *  - `changed`: rule 2, it holds a value that is not the default;
 *  - `revealed`: a deep link named it (or it was drawn earlier in this visit). */
export type Why = "mode" | "problem" | "changed" | "revealed";

export type VisCtx = {
  mode: Mode;
  settings: Settings;
  /** `DEFAULTS`, handed in so this file imports no value. */
  defaults: Settings;
  /** True where a host value differs from its default; absent = unknown. */
  host: Partial<Record<HostKey, boolean>>;
  /** Section ids with a problem (rule 3). */
  problems: ReadonlySet<string>;
  /** Which harness CLIs the host has; absent = not known yet, treated as
   *  installed so a page is never hidden on a guess. */
  installed: Partial<Record<Harness, boolean>>;
  /** Deep-linked, or drawn earlier in this visit (so a value set back to its
   *  default does not make its section vanish under the pointer). */
  revealed: ReadonlySet<string>;
  /** False on a window driving a paired host. */
  onHost: boolean;
};

const same = (a: unknown, b: unknown): boolean => {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") {
    // `undefined` and an absent key are the same thing in a merged blob.
    return (a === undefined && b === undefined);
  }
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a as object).filter((k) => (a as Record<string, unknown>)[k] !== undefined);
  const kb = Object.keys(b as object).filter((k) => (b as Record<string, unknown>)[k] !== undefined);
  if (ka.length !== kb.length) return false;
  return ka.every((k) => same((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
};

/** The tier a section is drawn at: a harness page's own section is Beginner
 *  only while that CLI is on the host (rule 4). */
export function effectiveTier(sec: SectionDef, ctx: Pick<VisCtx, "installed">): Mode {
  if (sec.harness && ctx.installed[sec.harness] === false && RANK[sec.tier] < RANK.advanced) return "advanced";
  return sec.tier;
}

/** Does `sec` hold a value that is not the default (rule 2)? */
export function sectionChanged(sec: SectionDef, ctx: Pick<VisCtx, "settings" | "defaults" | "host">): boolean {
  if (sec.keys?.some((k) => !same(ctx.settings[k], ctx.defaults[k]))) return true;
  return !!sec.host?.some((h) => ctx.host[h] === true);
}

/** Whether `sec` applies here at all (a remote window, the theme). A section
 *  that does not apply is neither drawn nor counted as hidden. */
export const sectionApplies = (sec: SectionDef, ctx: Pick<VisCtx, "settings" | "onHost">) =>
  !(sec.localOnly && !ctx.onHost) && !(sec.applies && !sec.applies(ctx.settings));

export function sectionWhy(sec: SectionDef, ctx: VisCtx): Why | null {
  if (!sectionApplies(sec, ctx)) return null;
  if (RANK[effectiveTier(sec, ctx)] <= RANK[ctx.mode]) return "mode";
  if (ctx.problems.has(sec.id)) return "problem";
  if (sectionChanged(sec, ctx)) return "changed";
  if (ctx.revealed.has(sec.id)) return "revealed";
  return null;
}

export const sectionsOn = (page: PageId) => SECTIONS.filter((s) => s.page === page);

/** Is the page listed in the nav? When any of its sections is drawn — which
 *  for a harness page whose CLI is on the host is always (rule 4 lives in
 *  `effectiveTier`: its own sections are Beginner then). */
export function pageListed(page: PageId, ctx: VisCtx): boolean {
  return sectionsOn(page).some((s) => sectionWhy(s, ctx) !== null);
}

/** The next mode up that would draw more, and how many more sections it
 *  would draw: "N more settings in Advanced ›". Counted over `pages`. */
export function hiddenBelow(pages: readonly PageId[], ctx: VisCtx): { count: number; next: Mode } | null {
  const hidden = SECTIONS.filter((s) => pages.includes(s.page) && sectionApplies(s, ctx) && sectionWhy(s, ctx) === null);
  if (hidden.length === 0) return null;
  for (const next of MODES.slice(RANK[ctx.mode] + 1)) {
    const count = hidden.filter((s) => RANK[effectiveTier(s, ctx)] <= RANK[next]).length;
    if (count > 0) return { count, next };
  }
  return null;
}

/** The footer a page carries (§4.3): its own hidden sections, plus — on the
 *  last page its group lists — those of the group's pages the mode does not
 *  list at all, so Beginner can always see that there is more. */
export function pageFooter(page: PageId, ctx: VisCtx): { count: number; next: Mode } | null {
  const group = PAGE[page].group;
  const pages = PAGES.filter((p) => p.group === group);
  const listed = pages.filter((p) => pageListed(p.id, ctx) || p.id === page);
  const last = listed[listed.length - 1]?.id === page;
  const unlisted = pages.filter((p) => !listed.includes(p)).map((p) => p.id);
  return hiddenBelow(last ? [page, ...unlisted] : [page], ctx);
}

// ── Reset, with a scope (§6 item 7) ─────────────────────────────────────────

export const RESET_SCOPES = [
  { id: "look", label: "Appearance & layout", hint: "Theme, sizes, fonts, the sidebar, the layout, the dock's width and tab, and the nav's sort." },
  { id: "behavior", label: "Window behaviour", hint: "Startup, commands, the default agent and models, chat view, notifications, pull requests, the file viewer's preferences, scrollback and the update check." },
  { id: "memory", label: "Per-place memory", hint: "Each place's panels, terminal tabs and their names, open files, collapsed docs, and each project's order and folding." },
  { id: "silenced", label: "Silenced suggestions", hint: "Every \"Stop suggesting this\", every dismissed init banner, and the dismissed update bubble — they can ask again." },
  { id: "above", label: "Settings above my mode", hint: "Every window setting this mode does not show, so nothing hidden keeps acting." },
  { id: "everything", label: "Everything", hint: "All of the above." },
] as const;
export type ResetScope = (typeof RESET_SCOPES)[number]["id"];

/** Keys Reset never touches, whatever the scope: the release-notes gate, the
 *  migration markers, and the sheet's own mode (resetting it would hide the
 *  section that holds the button). Host policy is not in the blob's DEFAULTS
 *  and is never reset from here. */
export const RESET_KEEPS = ["settings_rev", "last_seen_version", "chat_default_v1", "settings_mode"] as const;

const LOOK_PAGES: readonly PageId[] = ["appearance", "layout"];
const LOOK_SECTIONS = ["term-font"];
const LOOK_MEMORY = ["dock_open", "dock_width", "dock_tab", "dock_lower_frac", "sort_mode", "sort_dir"];

/** The keys a scope resets. Disjoint across look / behavior / memory /
 *  silenced, and `everything` is their union (`settingsmap-check.mjs`). */
export function resetKeys(scope: ResetScope, ctx: Pick<VisCtx, "mode" | "installed">): (keyof Settings)[] {
  const sectionKeys = (pred: (s: SectionDef) => boolean) => SECTIONS.filter(pred).flatMap((s) => s.keys ?? []);
  const look = () => [...sectionKeys((s) => LOOK_PAGES.includes(s.page) || LOOK_SECTIONS.includes(s.id)), ...LOOK_MEMORY] as (keyof Settings)[];
  const behavior = () => [
    ...sectionKeys((s) => !LOOK_PAGES.includes(s.page) && !LOOK_SECTIONS.includes(s.id)),
    ...MEMORY_KEYS.filter((k) => !LOOK_MEMORY.includes(k)),
  ] as (keyof Settings)[];
  const memory = () => [...PLACE_MEMORY_KEYS] as (keyof Settings)[];
  const silenced = () => ["offers_dismissed", "init_dismissed", "update_dismissed"] as (keyof Settings)[];
  switch (scope) {
    case "look": return look();
    case "behavior": return behavior();
    case "memory": return memory();
    case "silenced": return silenced();
    case "above": return sectionKeys((s) => RANK[effectiveTier(s, ctx)] > RANK[ctx.mode]);
    case "everything": return [...look(), ...behavior(), ...memory(), ...silenced()];
  }
}

/** The patch a Reset writes: each key back to its default. */
export function resetPatch(scope: ResetScope, ctx: Pick<VisCtx, "mode" | "installed" | "defaults">): Partial<Settings> {
  const keep = new Set<string>(RESET_KEEPS);
  return Object.fromEntries(resetKeys(scope, ctx).filter((k) => !keep.has(k)).map((k) => [k, ctx.defaults[k]])) as Partial<Settings>;
}
