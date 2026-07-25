// Persisted UI settings. Backed 1:1 by the get_settings/set_settings commands
// (the mock tolerates both; real backend writes ui-state.json in app-config-dir).
// Every visual setting is a CSS custom property, so applying settings is just
// variable assignment — no component re-render logic.
import { invoke } from "@tauri-apps/api/core";

export type Settings = {
  ui_rem: number; // 13–18
  term_family: string;
  term_size: number; // 10–20
  theme: "dark";
  density: "comfortable" | "compact";
  window_w: number;
  window_h: number;
  nav_width: number; // 220–460
  lens: "places" | "recent" | "attention";
  collapsed: Record<string, boolean>; // per-project-root collapse
};

export const DEFAULTS: Settings = {
  ui_rem: 15,
  term_family: '"SF Mono", Menlo, Monaco, monospace',
  term_size: 13,
  theme: "dark",
  density: "comfortable",
  window_w: 1280,
  window_h: 820,
  nav_width: 300,
  lens: "places",
  collapsed: {},
};

export const clampRem = (v: number) => Math.max(13, Math.min(18, v));
export const clampTerm = (v: number) => Math.max(10, Math.min(20, v));
export const clampNav = (v: number) => Math.max(220, Math.min(460, v));

/** Write the visual settings to the DOM as CSS vars / data-attrs. Cheap; safe to call often. */
export function applySettings(s: Settings) {
  const root = document.documentElement;
  root.style.setProperty("--ui-rem", `${clampRem(s.ui_rem)}px`);
  root.style.setProperty("--term-family", s.term_family);
  root.style.setProperty("--term-size", `${clampTerm(s.term_size)}px`);
  root.style.setProperty("--nav-w", `${clampNav(s.nav_width)}px`);
  root.dataset.theme = s.theme;
  root.dataset.density = s.density;
}

export async function loadSettings(): Promise<Settings> {
  try {
    const raw = await invoke<Partial<Settings> | null>("get_settings");
    return { ...DEFAULTS, ...(raw ?? {}) };
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
