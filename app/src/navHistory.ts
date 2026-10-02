// Back / forward through what the two panes showed — the RULES, pure.
//
// No React, no DOM, no imports: App.tsx observes the current location and
// hands it here; this module decides whether it is a new entry, a refinement of
// the current one, or nothing. `app/scripts/navhistory-check.mjs` evaluates
// this file as-is. Design: docs/proposals/nav-history.md.

export type DockTab = "files" | "terminal" | "docs" | "plan" | "automations";

/** What the panes show. `place: null` is Home. `dock.file` only means anything
 *  on the Files tab and `dock.shell` only on Terminal — `normalize` drops the
 *  other, so two locations that LOOK the same compare equal. */
export type Loc = {
  place: { repo: string; slug: string } | null;
  agent?: string;
  dock?: { tab: DockTab; file?: { path: string; line?: number; col?: number }; shell?: number };
};

export type History = {
  entries: Loc[];
  index: number;
  /** When the current entry was PUSHED (ms), 0 once anything else moved it.
   *  A push within COALESCE_MS of it replaces it instead of stacking. */
  at: number;
};

export const CAP = 100;
export const COALESCE_MS = 600;

export const empty = (): History => ({ entries: [], index: -1, at: 0 });

export const normalize = (l: Loc): Loc => {
  if (!l.place) return { place: null };
  const out: Loc = { place: { repo: l.place.repo, slug: l.place.slug } };
  if (l.agent) out.agent = l.agent;
  if (l.dock) {
    const d: NonNullable<Loc["dock"]> = { tab: l.dock.tab };
    if (l.dock.tab === "files" && l.dock.file) {
      const f = l.dock.file;
      d.file = { path: f.path, ...(f.line ? { line: f.line } : {}), ...(f.line && f.col ? { col: f.col } : {}) };
    }
    if (l.dock.tab === "terminal" && l.dock.shell != null) d.shell = l.dock.shell;
    out.dock = d;
  }
  return out;
};

/** Identity of a location — what "the same place twice" means. */
export const locKey = (l: Loc): string => JSON.stringify(normalize(l));

export const current = (h: History): Loc | null => h.entries[h.index] ?? null;

/** Drop adjacent duplicates (a removal or a coalesce can make two meet),
 *  keeping `index` on the same entry or the nearest one before it. */
const dedupe = (entries: Loc[], index: number): { entries: Loc[]; index: number } => {
  const out: Loc[] = [];
  let at = -1;
  entries.forEach((e, i) => {
    if (out.length && locKey(out[out.length - 1]) === locKey(e)) { if (i <= index) at = out.length - 1; return; }
    out.push(e);
    if (i <= index) at = out.length - 1;
  });
  return { entries: out, index: out.length ? Math.max(at, 0) : -1 };
};

/** Record what the panes show now.
 *
 *  `push` — a navigation: truncates forward, appends, unless it is the same
 *  location (nothing) or lands within COALESCE_MS of the last push (replaces
 *  that entry: "click a place → its remembered file opens 50 ms later" is one
 *  visit, and so is arrowing through nav rows to the one you stop on).
 *  `amend` — the app moved on its own (a restore, a removed place, a dead
 *  shell, history applying an entry): the current entry becomes this, forward
 *  is kept. */
export const record = (h: History, loc: Loc, now: number, mode: "push" | "amend" = "push"): History => {
  const l = normalize(loc);
  const cur = current(h);
  if (!cur) return { entries: [l], index: 0, at: mode === "push" ? now : 0 };
  if (locKey(cur) === locKey(l)) return h;
  if (mode === "amend" || (h.at && now - h.at < COALESCE_MS)) {
    const entries = mode === "amend" ? h.entries.slice() : h.entries.slice(0, h.index + 1);
    entries[h.index] = l;
    const d = dedupe(entries, h.index);
    // A coalesce slides the window (now), so a steady run of quick steps folds
    // into the one you stop on; an amend is not a navigation and leaves it.
    return { ...d, at: mode === "amend" ? h.at : now };
  }
  let entries = [...h.entries.slice(0, h.index + 1), l];
  if (entries.length > CAP) entries = entries.slice(entries.length - CAP);
  return { entries, index: entries.length - 1, at: now };
};

/** Indices reachable in `dir` (nearest first), skipping entries `alive`
 *  rejects — a place removed elsewhere is not somewhere you can go back to. */
export const reachable = (h: History, dir: -1 | 1, alive: (l: Loc) => boolean = () => true): number[] => {
  const out: number[] = [];
  for (let i = h.index + dir; i >= 0 && i < h.entries.length; i += dir) if (alive(h.entries[i])) out.push(i);
  return out;
};

export const canStep = (h: History, dir: -1 | 1, alive?: (l: Loc) => boolean): boolean =>
  reachable(h, dir, alive).length > 0;

/** Move to entry `i`. Clears `at`, so the observation that follows the apply
 *  can never coalesce into the entry you came FROM. */
export const goTo = (h: History, i: number): History =>
  i < 0 || i >= h.entries.length ? h : { ...h, index: i, at: 0 };

export const step = (h: History, dir: -1 | 1, alive?: (l: Loc) => boolean): History => {
  const [i] = reachable(h, dir, alive);
  return i === undefined ? h : goTo(h, i);
};

/** Forget entries `dead` matches (a removed place, a removed project). The
 *  current entry, if it goes, hands over to the nearest survivor before it. */
export const prune = (h: History, dead: (l: Loc) => boolean): History => {
  const keep: Loc[] = [];
  let index = -1;
  h.entries.forEach((e, i) => {
    if (dead(e)) return;
    keep.push(e);
    if (i <= h.index) index = keep.length - 1;
  });
  if (keep.length === h.entries.length) return h;
  const d = dedupe(keep, Math.max(index, 0));
  return { ...d, at: h.index >= 0 && !dead(h.entries[h.index]) ? h.at : 0 };
};

const base = (p: string) => p.slice(p.lastIndexOf("/") + 1);
const TAB_LABEL: Record<DockTab, string> = { files: "Files", terminal: "Terminal", docs: "Docs", plan: "Plan", automations: "Automations" };

/** One line for a tooltip or the history menu. `nameOf` turns a place into
 *  what the nav calls it (a title, `(main)`); the slug otherwise. */
export const label = (l: Loc, nameOf?: (repo: string, slug: string) => string): string => {
  if (!l.place) return "Home";
  const parts = [nameOf?.(l.place.repo, l.place.slug) ?? l.place.slug];
  const d = l.dock;
  if (d) {
    if (d.tab === "files" && d.file) parts.push(base(d.file.path) + (d.file.line ? `:${d.file.line}` : ""));
    else if (d.tab === "terminal" && d.shell != null) parts.push(`Terminal · sh ${d.shell}`);
    else parts.push(TAB_LABEL[d.tab]);
  }
  return parts.join(" · ");
};
