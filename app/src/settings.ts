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

/** The panel state a SPACE owns — what is open, which tab, how wide.
 *
 *  Deliberately just these three. The Files viewer's own preferences
 *  (`files_layout`, `files_split_pct`, `files_wrap`, `files_md_source`,
 *  `files_show_ignored`) stay GLOBAL: they are how you like to read a file, not
 *  which panel a place has open. The nav stays global for the same reason in
 *  reverse — it is how you LEAVE a space, so it must not move under you when
 *  you arrive somewhere.
 *
 *  The open FILE is not here either. A remembered path can be deleted, renamed
 *  or gitignored between visits, which turns "restore what I left" into an
 *  error banner on arrival. */
export type PlacePanels = {
  dock_open: boolean;
  dock_tab: "files" | "terminal";
  dock_width: number;
};

/** Key for the per-place records in `Settings` (`place_panels`,
 *  `term_tab_names`). A place is identified by the PAIR — slugs are only unique
 *  within a project. */
export const placeKey = (repo: string, slug: string) => `${repo}|${slug}`;

/** Settings as they apply to ONE place: the place's own remembered panels win
 *  where present, and a place with no entry starts with the dock CLOSED. */
export function panelsFor(s: Settings, key: string | null): Settings {
  const p = key ? s.place_panels?.[key] : undefined;
  if (p) return { ...s, ...p };
  // A place you have never opened the dock in starts CLOSED — the global
  // `dock_open` is NOT a seed.
  //
  // It used to be, on the theory that arriving somewhere new should not
  // rearrange the furniture. In practice that made the dock SPREAD: open it in
  // one place and every place you visited afterwards was already open before
  // you asked for it, which is the same "it changed what I left it in"
  // complaint pointed the other way. "No entry" has to mean "not set up yet",
  // not "whatever the last place was set to".
  //
  // `dock_tab` and `dock_width` still seed from the globals: neither is visible
  // until the dock is open, so inheriting them only decides what the FIRST
  // deliberate open looks like, which is exactly where a last-used value is
  // the right guess.
  return s.dock_open ? { ...s, dock_open: false } : s;
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
  // Files tab: how the tree and the viewer sit relative to each other.
  // "auto" flips to side-by-side once the dock is at least SPLIT_AT wide — a
  // narrow dock has no room for two columns, a wide one wastes half its width
  // stacking. "stack"/"split" pin it.
  files_layout: "auto" | "stack" | "split";
  files_split_pct: number; // 15–85, the tree's WIDTH share when side-by-side
  files_stack_pct: number; // 15–85, the tree's HEIGHT share when stacked
  files_wrap: boolean; // wrap long lines in the source view
  files_md_source: boolean; // markdown/SVG: show source instead of the render
  files_show_ignored: boolean; // list gitignored entries too (dimmed), .git aside; on by default
  // Dock terminal tab names: `repo|slug` → tab index → user-chosen label.
  // Only the NAME persists across restarts, never the shell: a named tab with no
  // live shell is seeded back into the strip and spawns a fresh shell when you
  // activate it. Closing a tab explicitly drops its name.
  term_tab_names: Record<string, Record<number, string>>;
  // Per-place panel state, keyed `repo|slug` (the same scheme as
  // `term_tab_names`). An entry here means "this place has been SET UP"; its
  // absence means the dock has never been opened there, and such a place starts
  // CLOSED — see `panelsFor`, which deliberately does not let the global
  // `dock_open` seed it. The flat `dock_tab`/`dock_width` DO still seed, since
  // neither is visible until the dock is open. Entries persist across restarts
  // and are pruned when a place or a project goes away.
  place_panels: Record<string, PlacePanels>;
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
  // Bumped when a DEFAULT changes in a way that has to reach installs which
  // already persisted the old value. Without it a default flip is invisible to
  // exactly the people who have been running the app — see `migrate`.
  settings_rev: number;
};

/** Current settings revision. Bump ONLY together with a step in `migrate`. */
export const SETTINGS_REV = 1;

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
  files_layout: "auto",
  files_split_pct: 32,
  files_stack_pct: 40,
  files_wrap: false,
  files_md_source: false,
  files_show_ignored: true,
  term_tab_names: {},
  place_panels: {},
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
  settings_rev: SETTINGS_REV,
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
 * the badges retire — they're duplicated in the nav row and the status bar.
 * Measured against the SPACE HEADER's width (main + dock), not `mainW`: the
 * header spans both columns, so a wide dock gives it room the center pane
 * alone does not have. */
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
  return { navShown, navW, dockShown, dockW, mainW, tight: mainW + dockW < MAIN_TIGHT };
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

/** Re-apply defaults that changed since the persisted revision. Mutates `s`;
 *  returns whether anything moved, which is also whether it must be SAVED —
 *  an unsaved migration re-runs on every launch and would keep overwriting a
 *  deliberate later choice.
 *
 *  rev 1 — `files_show_ignored` flipped to true. A tree that withholds entries
 *  with nothing on screen admitting it does not read as a filter, it reads as a
 *  broken listing: the files a session had just written were simply absent. */
function migrate(s: Settings, from: number): boolean {
  if (from >= SETTINGS_REV) return false;
  if (from < 1) s.files_show_ignored = DEFAULTS.files_show_ignored;
  s.settings_rev = SETTINGS_REV;
  return true;
}

export async function loadSettings(): Promise<Settings> {
  try {
    const raw = await invoke<Partial<Settings> | null>("get_settings");
    const s = { ...DEFAULTS, ...(raw ?? {}) };
    s.theme = normalizeTheme(s.theme);
    s.theme_light = normalizePair(s.theme_light, "light", DEFAULTS.theme_light);
    s.theme_dark = normalizePair(s.theme_dark, "dark", DEFAULTS.theme_dark);
    if (migrate(s, typeof raw?.settings_rev === "number" ? raw.settings_rev : 0)) saveSettings(s);
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
