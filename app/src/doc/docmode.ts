// Edit or Read, per KIND of document (workspace-view-design.md §4.2): a plan
// and a workspace's memory open in Read, an idea in Edit, and the last choice
// made for a kind is the one its next document opens in. A convenience of this
// machine's two windows, so it is browser storage, which may be missing or
// empty: then the defaults hold.
export type DocKind = "idea" | "plan" | "memory";
export type DocMode = "edit" | "read";

const KEY = "wt.doc.mode.";

/** What a kind opens in before anyone has chosen. */
export const defaultMode = (kind: DocKind): DocMode => (kind === "idea" ? "edit" : "read");

export function startMode(kind: DocKind): DocMode {
  try {
    const v = window.localStorage.getItem(KEY + kind);
    if (v === "edit" || v === "read") return v;
  } catch { /* no storage */ }
  return defaultMode(kind);
}

export function rememberMode(kind: DocKind, mode: DocMode): void {
  try { window.localStorage.setItem(KEY + kind, mode); } catch { /* no storage */ }
}
