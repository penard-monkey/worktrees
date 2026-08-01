// Persisted UI settings. Backed 1:1 by the get_settings/set_settings commands
// (the mock tolerates both; real backend writes ui-state.json in app-config-dir).
// Every visual setting is a CSS custom property, so applying settings is just
// variable assignment — no component re-render logic.
import { invoke } from "@tauri-apps/api/core";

// Shippable themes: each id is a [data-theme] color map in tokens.css.
// "system" follows macOS appearance and resolves to the Tokyo Night pair.
export const THEMES = [
  { id: "tokyo-night", label: "Tokyo Night", appearance: "dark" },
  { id: "tokyo-day", label: "Tokyo Night Day", appearance: "light" },
  { id: "catppuccin-mocha", label: "Catppuccin Mocha", appearance: "dark" },
  { id: "catppuccin-latte", label: "Catppuccin Latte", appearance: "light" },
  { id: "nord", label: "Nord", appearance: "dark" },
  { id: "gruvbox-dark", label: "Gruvbox Dark", appearance: "dark" },
] as const;
export type ThemeId = (typeof THEMES)[number]["id"];
export type ThemeSetting = ThemeId | "system";

/** Concrete [data-theme] value for a setting ("system" → macOS appearance,
 * flipping between the user's chosen light/dark pair). */
export function resolveTheme(s: Pick<Settings, "theme" | "theme_light" | "theme_dark">): ThemeId {
  if (s.theme === "system")
    return window.matchMedia("(prefers-color-scheme: light)").matches ? s.theme_light : s.theme_dark;
  return s.theme;
}

export type Settings = {
  ui_rem: number; // 13–18
  term_family: string;
  term_size: number; // 10–20
  theme: ThemeSetting;
  theme_light: ThemeId; // the pair "system" flips between
  theme_dark: ThemeId;
  density: "comfortable" | "compact";
  nav_width: number; // 220–460, further capped by the viewport (see fitLayout)
  nav_guides: boolean; // draw the tree plumb lines in the nav (data-guides on <html>)
  nav_collapsed: boolean; // rail-only mode (nav hidden)
  dock_open: boolean; // right dock (Files / Terminal) visible for the selected place
  dock_width: number; // ≥240, ceiling is viewport-derived (see dockCeiling)
  dock_tab: "files" | "terminal"; // last-used dock tab
  // Dock terminal tab names: `repo|slug` → tab index → user-chosen label.
  // Only the NAME persists across restarts, never the shell: a named tab with no
  // live shell is seeded back into the strip and spawns a fresh shell when you
  // activate it. Closing a tab explicitly drops its name.
  term_tab_names: Record<string, Record<number, string>>;
  editor_cmd: string; // "Open in editor" command, e.g. code / cursor / subl
  terminal_cmd: string; // "Open in terminal app" command; {session} → shell-quoted tmux session. "" hides the menu item.
  ai_auto_resume: boolean; // single-click Enter resumes an existing Claude conversation (Claude only)
  update_auto_check: boolean; // check for updates ~3s after launch (manual check always works)
  fetch_interval_min: number; // background `git fetch origin` cadence (0 = off, else 5 | 15 | 60)
  restore_last: boolean; // on launch, SELECT the most recently opened place (selection-only, never enters)
  lens: "places" | "recent" | "attention";
  collapsed: Record<string, boolean>; // per-project-root collapse
  hidden_tiers: string[]; // lifecycle tiers hidden in the Places lens (active/idle/dormant)
  sort_mode: "recent" | "alpha" | "manual";
  sort_dir: "asc" | "desc";
  manual_order: Record<string, string[]>; // repo root -> slug order (Manual sort)
  last_seen_version: string; // release-notes gate: "" = fresh install (record silently)
  // §9's init nudge, dismissed per project root. The VALUE is the suggestion's
  // content hash, never a boolean: a repo that later gains a credential file
  // produces a new hash and correctly re-suggests — the file that appears after
  // you dismissed the banner is precisely the one nobody links.
  //
  // ⚠ Two independent dismissal stores exist for this one concept, and that is
  // ACCEPTED rather than accidental (§9 asked for an explicit decision): the CLI
  // keeps its own marker under $XDG_STATE_HOME/worktrees/init-hints/ because
  // ui-state.json lives in the app's config dir and is invisible to it. They
  // gate different surfaces (a `new` hint line vs. this nav banner), they use
  // the same re-suggest rule, and unifying them would mean the CLI reading an
  // app-owned file — a worse dependency than a duplicated boolean-shaped fact.
  init_dismissed: Record<string, string>;
};

export const DEFAULTS: Settings = {
  ui_rem: 15,
  term_family: '"SF Mono", Menlo, Monaco, monospace',
  term_size: 13,
  theme: "tokyo-night",
  theme_light: "tokyo-day",
  theme_dark: "tokyo-night",
  density: "comfortable",
  nav_width: 300,
  nav_guides: true,
  nav_collapsed: false,
  dock_open: false,
  dock_width: 360,
  dock_tab: "files",
  term_tab_names: {},
  editor_cmd: "code",
  terminal_cmd: "",
  ai_auto_resume: true,
  update_auto_check: true,
  fetch_interval_min: 0,
  restore_last: false,
  lens: "places",
  collapsed: {},
  hidden_tiers: [],
  sort_mode: "recent",
  sort_dir: "desc",
  manual_order: {},
  last_seen_version: "",
  init_dismissed: {},
};

/** check_update result — versions the Settings "Version" section renders. */
export type UpdateInfo = {
  app_version: string;
  cli_version: string | null;
  cli_path: string | null;
  latest: string | null; // release tag, e.g. "v0.2.0"; null offline / no releases
};

export const clampRem = (v: number) => Math.max(13, Math.min(18, v));
export const clampTerm = (v: number) => Math.max(10, Math.min(20, v));

// ── column geometry ─────────────────────────────────────────────────────────
// The window is rail + nav + main + dock + rail. Only `main` is elastic, so
// both panels need a ceiling derived from the CURRENT viewport — a flat cap either
// strands space on a big screen (the old dock cap of 680 left ~470px unusable
// when fullscreen) or overflows a small one, which is what made the topbar's
// flex children pile on top of each other.
export const RAIL_W = 44; // keep in sync with --rail-w (tokens.css)
// Both rails are permanent columns — the left one picks the lens, the right one
// picks the dock tab — so the elastic center is short two of them, always.
export const RAILS_W = RAIL_W * 2;
export const MAIN_MIN = 420; // center-pane floor: below it the topbar can't lay out
export const NAV_MIN = 220;
export const NAV_MAX = 460;
export const DOCK_MIN = 240;

/** Usable width for the columns. clientWidth, not innerWidth: innerWidth counts
 * the scrollbar gutter, which silently eats MAIN_MIN's reserve (~15px) in a
 * browser harness. Single definition — App tracks it in state via this too. */
export const viewportWidth = () =>
  typeof document === "undefined" ? 1440 : document.documentElement.clientWidth || window.innerWidth;
const viewport = viewportWidth;

/** Widest the nav may be while `dockW` of dock is on screen. */
export const navCeiling = (dockW = 0, w = viewport()) =>
  Math.max(NAV_MIN, Math.min(NAV_MAX, w - RAILS_W - MAIN_MIN - dockW));
/** Widest the dock may be while `navW` of nav is on screen. */
export const dockCeiling = (navW = 0, w = viewport()) =>
  Math.max(DOCK_MIN, w - RAILS_W - navW - MAIN_MIN);

export const clampNav = (v: number, dockW = 0, w = viewport()) =>
  Math.max(NAV_MIN, Math.min(navCeiling(dockW, w), v));
export const clampDock = (v: number, navW = 0, w = viewport()) =>
  Math.max(DOCK_MIN, Math.min(dockCeiling(navW, w), v));

/** What the columns actually render as right now. Preferences in `Settings` are
 * INTENT and are never rewritten by fitting — a window that gets too narrow
 * hides the dock, and growing it back brings the dock back, because `dock_open`
 * still says "open". Degrade order: shrink the dock, then drop it, then let the
 * nav take its own (already viewport-capped) width. */
export type Fit = { navShown: boolean; navW: number; dockShown: boolean; dockW: number; mainW: number; tight: boolean };

/** Below this the topbar can't hold the status badges AND a readable slug, so
 * the badges retire — they're duplicated in the nav row and the status bar. */
export const MAIN_TIGHT = 560;

export function fitLayout(s: Settings, dockEligible: boolean, w = viewport()): Fit {
  const navShown = !s.nav_collapsed;
  let dockShown = s.dock_open && dockEligible;
  // reserve the dock's floor while sizing the nav, so the two can't both win
  let navW = navShown ? clampNav(s.nav_width, dockShown ? DOCK_MIN : 0, w) : 0;
  let dockW = dockShown ? clampDock(s.dock_width, navW, w) : 0;
  if (dockShown && w - RAILS_W - navW - dockW < MAIN_MIN) {
    // both floors together don't fit (possible at the 900px minWidth) — the
    // dock yields, and the nav reclaims the width it lent to the reservation
    dockShown = false;
    dockW = 0;
    navW = navShown ? clampNav(s.nav_width, 0, w) : 0;
  }
  const mainW = w - RAILS_W - navW - dockW;
  return { navShown, navW, dockShown, dockW, mainW, tight: mainW < MAIN_TIGHT };
}

/** Write the visual settings to the DOM as CSS vars / data-attrs. Cheap; safe to call often.
 * Column widths are NOT written here — they depend on the viewport and on
 * whether a place is selected, so App owns them (see the fitLayout effect). */
export function applySettings(s: Settings) {
  const root = document.documentElement;
  root.style.setProperty("--ui-rem", `${clampRem(s.ui_rem)}px`);
  root.style.setProperty("--term-family", s.term_family);
  root.style.setProperty("--term-size", `${clampTerm(s.term_size)}px`);
  root.dataset.theme = resolveTheme(s);
  root.dataset.density = s.density;
  root.dataset.guides = s.nav_guides ? "on" : "off";
}

// Pre-theme installs persisted theme:"dark"; unknown ids (downgrades) reset too.
function normalizeTheme(t: unknown): ThemeSetting {
  if (t === "system" || THEMES.some((x) => x.id === t)) return t as ThemeSetting;
  return DEFAULTS.theme;
}
// System-pair pickers only accept a theme of the right appearance.
function normalizePair(t: unknown, appearance: "light" | "dark", fallback: ThemeId): ThemeId {
  const hit = THEMES.find((x) => x.id === t && x.appearance === appearance);
  return hit ? hit.id : fallback;
}

export async function loadSettings(): Promise<Settings> {
  try {
    const raw = await invoke<Partial<Settings> | null>("get_settings");
    const s = { ...DEFAULTS, ...(raw ?? {}) };
    s.theme = normalizeTheme(s.theme);
    s.theme_light = normalizePair(s.theme_light, "light", DEFAULTS.theme_light);
    s.theme_dark = normalizePair(s.theme_dark, "dark", DEFAULTS.theme_dark);
    return s;
  } catch {
    return { ...DEFAULTS };
  }
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;
export function saveSettings(s: Settings) {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    invoke("set_settings", { settings: s }).catch(() => {
      /* harness / offline — ignore */
    });
  }, 250);
}
