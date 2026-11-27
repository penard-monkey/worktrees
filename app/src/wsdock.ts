// The dock in a WORKSPACE's view (the owner's ask of 2026-11-27: "when in the
// chat view the terminal should work … always at ~/ … remembered like the
// other ones").
//
// A workspace has no worktree, so of the dock's tabs only the Terminal means
// anything there: shells that start in the home directory, one strip per
// workspace. Its shells are the backend's ordinary dock shells under a scope
// no project can have — `repo` is `WORKSPACE_SCOPE`, `slug` the workspace's id
// (`term::WORKSPACE_SCOPE` in worktrees-api, which serve refuses to every
// paired device) — so every per-place record (`term_tabs`, `term_tab_names`,
// `term_tab_active`, `place_panels`) keys it `~|<id>` like any place.
//
// Pure, so `wsdock-check.mjs` can hold the rule without rendering App.

import type { Settings } from "./settings";

/** The `repo` of a workspace's dock shells. Mirrors `term::WORKSPACE_SCOPE`;
 *  a project's repo is always an absolute path, so the two never meet. */
export const WORKSPACE_SCOPE = "~";

/** The record key of workspace `id`'s dock: its shells, tabs and panels. */
export const wsDockKey = (id: string) => `${WORKSPACE_SCOPE}|${id}`;

/** Is `key` a workspace's dock record (and whose), rather than a place's? */
export const wsOfDockKey = (key: string): string | null =>
  key.startsWith(WORKSPACE_SCOPE + "|") ? key.slice(WORKSPACE_SCOPE.length + 1) : null;

/** Where the dock is being asked about: a place on screen, a workspace's view
 *  on screen (and no place), and whether the window has room for it. */
export type DockAt = { place: boolean; workspace: boolean; fits: boolean };

/** May the dock show at all here. */
export const dockEligibleAt = (at: DockAt) => at.place || at.workspace;

/** The tabs a workspace's dock offers. Files, Docs, Plan, PRs and Workspace are
 *  a place's: they need a worktree. */
export const WS_DOCK_TABS: readonly Settings["dock_tab"][] = ["terminal"];

/** Why the rail's `key` button is disabled here, or null when it is live. */
export function dockWhyFor(key: Settings["dock_tab"], at: DockAt): string | null {
  if (!at.place && !(at.workspace && WS_DOCK_TABS.includes(key))) return "select a place first";
  return at.fits ? null : "window too narrow";
}

/** The tab the dock shows: a workspace's is always its Terminal, whatever the
 *  last place left in `dock_tab` (which a workspace never writes). */
export const dockTabAt = (at: Omit<DockAt, "fits">, placeTab: Settings["dock_tab"]): Settings["dock_tab"] =>
  !at.place && at.workspace ? "terminal" : placeTab;
