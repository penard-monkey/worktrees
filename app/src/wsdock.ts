// Workspace shells reuse the ordinary dock transport under a reserved scope.
import type { PlacePanels } from "./settings";

/** The `repo` of a workspace's dock shells. Mirrors `term::WORKSPACE_SCOPE`;
 *  a project's repo is always an absolute path, so the two never meet. */
export const WORKSPACE_SCOPE = "~";

/** The record key of workspace `id`'s dock: its shells, tabs and panels. */
export const wsDockKey = (id: string) => `${WORKSPACE_SCOPE}|${id}`;

/** Is `key` a workspace's dock record (and whose), rather than a place's? */
export const wsOfDockKey = (key: string): string | null =>
  key.startsWith(WORKSPACE_SCOPE + "|") ? key.slice(WORKSPACE_SCOPE.length + 1) : null;

/** The workspace's saved choice, including records from the Terminal-only dock. */
export const workspaceTerminalSelected = (stored?: PlacePanels): boolean =>
  stored?.workspace_terminal ?? (stored?.dock_tab === "terminal");
