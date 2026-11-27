// Rename in the nav: the rules, with no React and no imports, so
// `rowname-check.mjs` loads this file as it is.
//
// One gesture set for every nameable row (a lane, a workspace chat, a
// workspace header): the row's right-click menu has Rename, and F2 renames
// the row on screen. The edit is an inline field: Enter and blur save, Escape
// cancels. Renaming changes the display title only, never a slug, a branch,
// a session or a chat's id.

/** What an edit asks for: nothing, or a new title. "" asks to clear a title
 *  (a lane goes back to its slug). */
export type NameChange = { kind: "none" } | { kind: "set"; title: string };

/** A lane's field holds the name on screen (its title, else its slug). An
 *  empty field, or the slug typed back, clears the title; the same name is
 *  no change. */
export function laneChange(typed: string, title: string | undefined, slug: string): NameChange {
  const v = typed.trim();
  const had = title?.trim() ?? "";
  if (v === "" || v === slug) return had ? { kind: "set", title: "" } : { kind: "none" };
  return v === had ? { kind: "none" } : { kind: "set", title: v };
}

/** A chat or a workspace always has a name: an empty field is a cancel, never
 *  an error, and the same name is no change. */
export function namedChange(typed: string, current: string): NameChange {
  const v = typed.trim();
  return v === "" || v === current.trim() ? { kind: "none" } : { kind: "set", title: v };
}

/** Is this F2 one for the nav? Plain F2 only, and never while a text field
 *  owns the keyboard (a field's own F2 is its business). */
export function isRenameKey(e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; repeat: boolean }, inField: boolean): boolean {
  return e.key === "F2" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.repeat && !inField;
}
