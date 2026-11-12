// Ideas relate (docs/proposals/idea-board-phases.md §3, §6.1, phase 3): the
// rules the window follows, pure. No import at runtime: ideas-relate-check.mjs
// loads this file as a `data:` module and runs every non-drag path through it,
// since a check cannot drive a real drag (§6.1). The drag and its menu path
// end in the SAME call (`relateCall`, `mergeCall`, `combineCall`,
// `splitCall`), which is the row a later agent proposal makes too (§7.1).

/** An idea's id: a ULID, 26 Crockford base-32 characters. */
export const ULID = /^[0-9A-HJKMNP-TV-Z]{26}$/;

/** What a row needs to know about an idea to take part. */
export type Row = { id: string | null; title: string; problem?: string };

/** One row call: its command and its arguments, as `invoke` takes them. */
export type Call = { cmd: string; args: Record<string, unknown> };

/** What dropping one idea row onto another may do; `null` = not a target,
 *  which shows no highlight and does nothing (§6.1's drop rules). */
export function ideaDrop(drag: string, onto: Row | null): string | null {
  if (!onto || !onto.id || onto.problem || onto.id === drag) return null;
  return onto.id;
}

/** Dropping selected rows onto "+ New idea": combine, when two or more of
 *  them are ideas that can be acted on. */
export function combineDrop(ids: readonly string[]): string[] | null {
  const uniq = [...new Set(ids.filter((i) => ULID.test(i)))];
  return uniq.length >= 2 ? uniq : null;
}

/** The chooser's answers, in order; the first is the default (Enter). */
export const CHOICES = ["relate", "merge"] as const;
export type Choice = (typeof CHOICES)[number];
/** Which answers ask first or offer Undo (§6.1: merge does, relate never). */
export const ASKS: Record<Choice, boolean> = { relate: false, merge: true };

export function relateCall(id: string, other: string): Call {
  return { cmd: "idea_relate", args: { id, other } };
}

/** `from` merged into `into`: the open one's mtime goes with it, so an edit
 *  made elsewhere since is refused rather than lost. */
export function mergeCall(into: string, from: string, mtimes: { into?: number | null; from?: number | null } = {}): Call {
  const args: Record<string, unknown> = { into, from };
  if (mtimes.into != null) args.expectedInto = mtimes.into;
  if (mtimes.from != null) args.expectedFrom = mtimes.from;
  return { cmd: "idea_merge", args };
}

export function combineCall(ids: readonly string[], title: string): Call {
  return { cmd: "idea_combine", args: { ids: [...ids], title: title.trim() || "Combined idea" } };
}

/** Split the selection `[start, end)` of the saved text. Offsets are what a
 *  JavaScript string counts (UTF-16), into the text `mtime` names — the core
 *  converts and refuses a stale one. ⌥ (or the menu's box) extracts. */
export function splitCall(id: string, start: number, end: number, extract: boolean, mtime: number, title?: string): Call {
  const args: Record<string, unknown> = { id, start, end, expectedMtime: mtime };
  if (extract) args.extract = true;
  if (title && title.trim()) args.title = title.trim();
  return { cmd: "idea_split", args };
}

/** The link a `[[` pick inserts: by id, so a rename is free (§3). */
export function ideaLink(title: string, id: string): string {
  const label = (title.trim() || "Untitled").replace(/\\/g, "\\\\").replace(/\[/g, "\\[").replace(/\]/g, "\\]");
  return `[${label}](idea:${id})`;
}

/** `[[query` ending at the cursor, on one line: the picker is open for it.
 *  `from` is where `[[` starts in `before` (the line up to the cursor). */
export function pickerQuery(before: string): { from: number; query: string } | null {
  const m = /\[\[([^\]\[\n]{0,80})$/.exec(before);
  return m ? { from: m.index, query: m[1] } : null;
}

/** The ideas a `[[` query offers: titles containing it (case-folded), the
 *  open idea left out, best first (a title that STARTS with it), at most 8. */
export function pickerMatches<T extends Row>(rows: readonly T[], query: string, exclude: string | null): T[] {
  const q = query.trim().toLowerCase();
  const ok = rows.filter((r) => r.id && !r.problem && r.id !== exclude && r.title.toLowerCase().includes(q));
  const starts = (r: T) => (r.title.toLowerCase().startsWith(q) ? 0 : 1);
  return [...ok].sort((a, b) => starts(a) - starts(b)).slice(0, 8);
}

/** The idea an `idea:` link at offset `at` of `line` points to, if any:
 *  inside `[text](idea:ID)` or on a bare `idea:ID`. */
export function linkAt(line: string, at: number): string | null {
  const re = /\[(?:\\.|[^\]\\])*\]\(idea:([0-9A-HJKMNP-TV-Z]{26})\)|idea:([0-9A-HJKMNP-TV-Z]{26})/g;
  for (let m = re.exec(line); m; m = re.exec(line)) {
    if (at >= m.index && at <= m.index + m[0].length) return m[1] ?? m[2];
  }
  return null;
}

/** The Read view's body with `idea:` links made `#idea:` ones, which the
 *  shared renderer allows (it refuses every scheme but http(s)) and hands to
 *  `onLink`; `ideaHref` reads one back. */
export function readableLinks(body: string): string {
  return body.replace(/\]\(idea:([0-9A-HJKMNP-TV-Z]{26})\)/g, "](#idea:$1)");
}
export function ideaHref(href: string): string | null {
  const m = /^#?idea:([0-9A-HJKMNP-TV-Z]{26})$/.exec(href);
  return m ? m[1] : null;
}

/** "B was merged into A on 12 Nov": the note an absorbed id lands with. */
export function mergedNote(m: { id: string; title?: string | null; date?: string | null }, into: string): string {
  const what = m.title ? `“${m.title}”` : "That idea";
  return `${what} was merged into “${into}”${m.date ? ` on ${m.date}` : ""}.`;
}

/** A relation's words, by how (`ideas::Relation::how`). */
export const HOW: Record<string, string> = {
  related: "related",
  "related-by": "related",
  "links-to": "linked",
  "linked-from": "links here",
  "split-from": "split from",
  "split-into": "split into",
};

/** The related list as the window shows it: one entry per idea, the first
 *  `how` the core gave (written relations come first there). */
export function relatedShown<T extends { id: string; how: string }>(rels: readonly T[]): T[] {
  const seen = new Set<string>();
  return rels.filter((r) => (seen.has(r.id) ? false : (seen.add(r.id), true)));
}

/** A history copy's time, short and local, for the Versions list. */
export function versionWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString([], { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

/** The selected rows after a click: plain picks one, ⌘/Ctrl toggles,
 *  Shift extends from the anchor through `order` (the list as shown). */
export function nextSelection(cur: readonly string[], id: string, mods: { toggle: boolean; range: boolean }, order: readonly string[], anchor: string | null): string[] {
  if (mods.range && anchor && order.includes(anchor) && order.includes(id)) {
    const [a, b] = [order.indexOf(anchor), order.indexOf(id)].sort((x, y) => x - y);
    return order.slice(a, b + 1);
  }
  if (mods.toggle) return cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
  return [id];
}
