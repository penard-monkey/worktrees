// Back / forward through what the two panes showed — the RULES, pure.
//
// No React, no DOM, no imports: App.tsx observes the current location and
// hands it here; this module decides whether it is a new entry, a refinement of
// the current one, or nothing. `app/scripts/navhistory-check.mjs` evaluates
// this file as-is. Design: docs/proposals/nav-history.md.

export type DockTab = "files" | "terminal" | "docs" | "plan" | "prs" | "workspace";

/** A workspace view's tabs (wsview.ts says which are drawn). */
export type WsTab = "overview" | "memory" | "plan" | "ideas" | "chats";

/** The Disk view over the panes (DiskView.tsx): which scope, and the file or
 *  folder picked in a place's treemap. Restated here, not imported from
 *  diskmodel.ts, because this module has no imports (the check loads it as a
 *  data: module). */
export type DiskScope =
  | { kind: "all" }
  | { kind: "workspace"; id: string }
  | { kind: "project"; root: string }
  | { kind: "place"; repo: string; slug: string };
export type DiskAt = { scope: DiskScope; path?: string };

/** What the panes show. `place: null` is Home, or the workspace view when `ws`
 *  names one: its id, its tab, on Ideas the idea picked (on Memory the file,
 *  and `hist` when its History is open: `at`, the change shown in it), and the
 *  chat in its middle column. `dock.file` only means anything on the Files
 *  tab, `dock.doc` (the document chosen in the list) only on Docs and
 *  `dock.shell` only on Terminal — `normalize` drops the others, so two
 *  locations that LOOK the same compare equal. */
export type Loc = {
  place: { repo: string; slug: string } | null;
  ws?: { id: string; tab: WsTab; doc?: string; chat?: string; hist?: { at?: string } };
  agent?: string;
  dock?: { tab: DockTab; file?: { path: string; line?: number; col?: number }; doc?: string; shell?: number };
  /** Disk, drawn over what the rest of the location shows. That rest stays a
   *  part of the entry: closing Disk goes back to it, and so does Back from the
   *  first Disk screen, because that is the entry before. */
  disk?: DiskAt;
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

const diskScope = (s: DiskScope): DiskScope =>
  s.kind === "place" ? { kind: "place", repo: s.repo, slug: s.slug }
    : s.kind === "project" ? { kind: "project", root: s.root }
    : s.kind === "workspace" ? { kind: "workspace", id: s.id } : { kind: "all" };
/** A place's treemap is the only scope with something to pick. */
const diskAt = (d: DiskAt): DiskAt => ({ scope: diskScope(d.scope), ...(d.path && d.scope.kind === "place" ? { path: d.path } : {}) });

export const normalize = (l: Loc): Loc => {
  const out = under(l);
  return l.disk ? { ...out, disk: diskAt(l.disk) } : out;
};
const under = (l: Loc): Loc => {
  if (!l.place) return l.ws ? { place: null, ws: { id: l.ws.id, tab: l.ws.tab, ...((l.ws.tab === "ideas" || l.ws.tab === "memory") && l.ws.doc ? { doc: l.ws.doc } : {}), ...(l.ws.chat ? { chat: l.ws.chat } : {}), ...(l.ws.tab === "memory" && l.ws.hist ? { hist: l.ws.hist.at ? { at: l.ws.hist.at } : {} } : {}) } } : { place: null };
  const out: Loc = { place: { repo: l.place.repo, slug: l.place.slug } };
  if (l.agent) out.agent = l.agent;
  if (l.dock) {
    const d: NonNullable<Loc["dock"]> = { tab: l.dock.tab };
    if (l.dock.tab === "files" && l.dock.file) {
      const f = l.dock.file;
      d.file = { path: f.path, ...(f.line ? { line: f.line } : {}), ...(f.line && f.col ? { col: f.col } : {}) };
    }
    if (l.dock.tab === "docs" && l.dock.doc) d.doc = l.dock.doc;
    if (l.dock.tab === "terminal" && l.dock.shell != null) d.shell = l.dock.shell;
    out.dock = d;
  }
  return out;
};

/** Identity of a location — what "the same place twice" means. */
export const locKey = (l: Loc): string => JSON.stringify(normalize(l));

/** WHERE a location is, whatever it shows there: the place, the workspace, or
 *  Home. What history waits to see land when it puts one back. */
export const whereKey = (l: Loc): string => (l.place ? `p|${l.place.repo}|${l.place.slug}` : l.ws ? `w|${l.ws.id}` : "home")
  + (l.disk ? `|disk|${JSON.stringify(diskScope(l.disk.scope))}` : "");

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
const TAB_LABEL: Record<DockTab, string> = { files: "Files", terminal: "Terminal", docs: "Docs", plan: "Plan", prs: "Pull requests", workspace: "Workspace" };

/** A tab's name, on the tab itself and wherever one is named. */
export const WS_TAB_LABEL: Record<WsTab, string> = { overview: "Overview", memory: "Memory", plan: "Plan", ideas: "Ideas", chats: "Chats" };

/** One line for a tooltip or the history menu. `nameOf` turns a place into
 *  what the nav calls it (a title, `(main)`); the slug otherwise. `wsName` is
 *  a workspace's name, `chatName` one of its chats'. */
export const label = (l: Loc, nameOf?: (repo: string, slug: string) => string, wsName?: (id: string) => string | null, chatName?: (id: string, conv: string) => string | null): string => {
  if (l.disk) {
    const s = l.disk.scope;
    const what = s.kind === "place" ? nameOf?.(s.repo, s.slug) ?? s.slug
      : s.kind === "project" ? base(s.root)
      : s.kind === "workspace" ? wsName?.(s.id) ?? "A workspace" : "All projects";
    return ["Disk", what, l.disk.path ? base(l.disk.path) : null].filter(Boolean).join(" · ");
  }
  if (!l.place) {
    if (!l.ws) return "Home";
    const chat = l.ws.chat ? chatName?.(l.ws.id, l.ws.chat) ?? "a chat" : null;
    const hist = l.ws.hist ? (l.ws.hist.at ? "A change in History" : "History") : null;
    return [wsName?.(l.ws.id) ?? "A workspace", chat, WS_TAB_LABEL[l.ws.tab], hist].filter(Boolean).join(" · ");
  }
  const parts = [nameOf?.(l.place.repo, l.place.slug) ?? l.place.slug];
  const d = l.dock;
  if (d) {
    if (d.tab === "files" && d.file) parts.push(base(d.file.path) + (d.file.line ? `:${d.file.line}` : ""));
    else if (d.tab === "terminal" && d.shell != null) parts.push(`Terminal · sh ${d.shell}`);
    else if (d.tab === "docs" && d.doc) parts.push(`Docs · ${base(d.doc)}`);
    // A bare Files tab is every place's default, dock open or not (⌘J is not
    // in the location), so naming it says nothing — the other tabs are news.
    else if (d.tab !== "files") parts.push(TAB_LABEL[d.tab]);
  }
  return parts.join(" · ");
};
