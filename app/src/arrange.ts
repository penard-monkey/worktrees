// Drag to arrange — the pure half. Which sections can be moved, where each one
// can go, what moving it WRITES, and where its drop zones sit on screen.
//
// Every destination is a patch over the three layout fields Settings → Layout
// already owns (`places_side`, `dock_side`, `dock_lower`). A drag is another
// way to make the same choices, never a new kind of state: nothing here can put
// a section somewhere the tiles cannot, so the grid builder in App.tsx has no
// new shape to learn, and Reset is "write the defaults of those three keys".
//
// No React, no DOM: App measures the rectangles and hands them in, so the
// zones can be checked under node (`layout-check.mjs`).
import type { Settings } from "./settings";

/** A section with a grip in its header. */
export type Grip = "places" | "reference" | "agent" | "terminal";

/** The fields a drop may write — and therefore what Undo restores and Reset
 *  defaults. Kept as a list so the three agree. */
export const LAYOUT_KEYS = ["places_side", "dock_side", "dock_lower"] as const;
export type LayoutPatch = Partial<Pick<Settings, (typeof LAYOUT_KEYS)[number]>>;
type LayoutState = Pick<Settings, (typeof LAYOUT_KEYS)[number]>;

export type Dest = {
  id: string;
  /** Menu item text. */
  label: string;
  /** Said by the aria-live region, and the Undo banner, after the move. */
  done: string;
  patch: LayoutPatch;
};

export const GRIP_LABEL: Record<Grip, string> = {
  places: "Places",
  reference: "Reference panels",
  agent: "Agent",
  terminal: "Terminal",
};

/** Where `g` can go. Two destinations each — the 2×2 and the lower slot,
 *  split by which section you are holding. */
export function destinations(g: Grip): Dest[] {
  switch (g) {
    case "places":
      return [
        { id: "places-left", label: "Left edge of the window", done: "Places moved to the left edge", patch: { places_side: "left" } },
        { id: "places-right", label: "Right edge of the window", done: "Places moved to the right edge", patch: { places_side: "right" } },
      ];
    case "reference":
      return [
        { id: "ref-left", label: "Left of the agent", done: "Reference panels moved to the left of the agent", patch: { dock_side: "left" } },
        { id: "ref-right", label: "Right of the agent", done: "Reference panels moved to the right of the agent", patch: { dock_side: "right" } },
      ];
    // The agent does not own a side of its own: it takes whichever half the
    // reference panels leave it. Dragging IT writes the reference's side.
    case "agent":
      return [
        { id: "agent-left", label: "Left of the reference panels", done: "Agent moved to the left of the reference panels", patch: { dock_side: "right" } },
        { id: "agent-right", label: "Right of the reference panels", done: "Agent moved to the right of the reference panels", patch: { dock_side: "left" } },
      ];
    case "terminal":
      return [
        { id: "term-tab", label: "In the panel's tabs", done: "Terminal moved into the panel's tabs", patch: { dock_lower: null } },
        { id: "term-below", label: "Below the reference panel", done: "Terminal moved below the reference panel", patch: { dock_lower: "terminal" } },
      ];
  }
}

/** The destination that is where the section already is. */
export const isCurrent = (d: Dest, s: LayoutState) =>
  (Object.keys(d.patch) as (keyof LayoutPatch)[]).every((k) => d.patch[k] === s[k]);

/** The values a patch is about to overwrite — Undo writes these back. */
export const undoOf = (p: LayoutPatch, s: LayoutState): LayoutPatch =>
  Object.fromEntries((Object.keys(p) as (keyof LayoutPatch)[]).map((k) => [k, s[k]])) as LayoutPatch;

/** The layout fields' defaults, for Reset. */
export const resetPatch = (defaults: LayoutState): LayoutPatch =>
  Object.fromEntries(LAYOUT_KEYS.map((k) => [k, defaults[k]])) as LayoutPatch;

export type Rect = { left: number; top: number; right: number; bottom: number };
export type Geometry = {
  /** The whole app grid (`.app`). */
  app: Rect;
  /** Agent + reference column, under the place header (`.space-body`). */
  space: Rect;
  /** The reference column, when it is on screen. */
  dock: Rect | null;
};

/** Each destination's drop zone, in viewport px. A zone is the region the
 *  section would occupy after the drop — roughly: an outer band of the window
 *  for Places, a half of the work area for the reference panels or the agent,
 *  a half of the reference column for the Terminal. Zones never overlap, so a
 *  pointer is over one zone or none. Null: that destination has no zone right
 *  now (the Terminal's, with the column closed). */
export function zoneOf(id: string, geo: Geometry): Rect | null {
  const { app, space, dock } = geo;
  const band = Math.min(320, Math.max(160, (app.right - app.left) * 0.22));
  const midX = (space.left + space.right) / 2;
  switch (id) {
    case "places-left": return { left: app.left, top: app.top, right: app.left + band, bottom: app.bottom };
    case "places-right": return { left: app.right - band, top: app.top, right: app.right, bottom: app.bottom };
    case "ref-left": case "agent-left": return { left: space.left, top: space.top, right: midX, bottom: space.bottom };
    case "ref-right": case "agent-right": return { left: midX, top: space.top, right: space.right, bottom: space.bottom };
    case "term-tab": case "term-below": {
      if (!dock) return null;
      const midY = (dock.top + dock.bottom) / 2;
      return id === "term-tab"
        ? { left: dock.left, top: dock.top, right: dock.right, bottom: midY }
        : { left: dock.left, top: midY, right: dock.right, bottom: dock.bottom };
    }
  }
  return null;
}

/** The destination under a point, if any. */
export function destAt(g: Grip, geo: Geometry, x: number, y: number): Dest | null {
  for (const d of destinations(g)) {
    const r = zoneOf(d.id, geo);
    if (r && x >= r.left && x < r.right && y >= r.top && y < r.bottom) return d;
  }
  return null;
}
