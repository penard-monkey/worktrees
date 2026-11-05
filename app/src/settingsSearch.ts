// The Settings header's search: which sections a query names, best first.
//
// PURE and import-free at runtime (type imports only, which esbuild erases),
// like settingsMap.ts: `settingssearch-check.mjs` loads both as data: modules
// and hands this one the real map, so the ranking is tested against the real
// sections and their real keywords with no second copy of either.
//
// What is indexed (the owner's choice): a section's label, its page's and its
// group's labels, the setting keys it owns, and its hand-written `keywords`.
// NOT the text inside its controls (option labels, hints) — that is what makes
// a search box return half the sheet for "on".
//
// The ranking is fixed and deterministic, best first:
//   name       the section's label starts with the query
//   word       a word of the label does (or each query word starts one)
//   synonym    one of its keywords does
//   context    its page's or group's label, or one of its keys, does — or the
//              query's words are spread over those and the label ("claude
//              policy" is Claude's Agent policy)
//   substring  the query (3+ characters) is inside any of the above
// and within a rank, sheet order (the rail's, top to bottom). The mode plays no
// part: the same query lists the same rows in every mode, and a row the mode
// hides says so with its tier chip (SettingsSearch.tsx).
import type { SectionDef } from "./settingsMap";

export const RANKS = ["name", "word", "synonym", "context", "substring"] as const;
export type Rank = (typeof RANKS)[number];

/** Rows the list shows at most: a list is a shortcut, not a results page. */
export const SEARCH_LIMIT = 8;
/** Shorter than this, a query matches word starts only: "on" inside every
 *  "session" and "options" is noise, not a hit. */
export const SUBSTRING_MIN = 3;

/** Lowercase words: "Light ↔ dark pair" → "light dark pair", "term_scrollback"
 *  → "term scrollback", "Claude in (main)" → "claude in main". */
export const norm = (s: string): string =>
  s.toLowerCase().normalize("NFKD").replace(/[̀-ͯ]/g, "").replace(/[^a-z0-9]+/g, " ").trim();

export type SearchDoc = {
  id: string;
  page: string;
  /** As the sheet prints it. */
  label: string;
  /** "Group › Page", or the one name where the two are the same. */
  where: string;
  /** Normalised fields, by rank. */
  name: string;
  keywords: readonly string[];
  context: readonly string[];
  /** Everything, joined: for a query whose words land in different fields. */
  all: string;
};

export type SearchHit = { id: string; page: string; label: string; where: string; rank: Rank };

type MapIn = {
  sections: readonly Pick<SectionDef, "id" | "page" | "label" | "keywords" | "keys" | "host">[];
  pages: readonly { id: string; label: string; group: string }[];
  groups: readonly { id: string; label: string }[];
};

/** The index, in sheet order. Built once per map; the map never changes at
 *  runtime, so the sheet builds it at module load. */
export function searchIndex({ sections, pages, groups }: MapIn): SearchDoc[] {
  const pageAt = (id: string) => pages.findIndex((p) => p.id === id);
  return sections
    .map((s, i) => ({ s, i }))
    .sort((a, b) => pageAt(a.s.page) - pageAt(b.s.page) || a.i - b.i)
    .map(({ s }) => {
      const page = pages[pageAt(s.page)];
      const group = groups.find((g) => g.id === page.group);
      const groupLabel = group?.label ?? page.group;
      const name = norm(s.label);
      const keywords = s.keywords.map(norm).filter(Boolean);
      const context = [page.label, groupLabel, ...(s.keys ?? []), ...(s.host ?? [])].map((x) => norm(String(x))).filter(Boolean);
      return {
        id: s.id, page: s.page, label: s.label,
        where: groupLabel === page.label ? page.label : `${groupLabel} › ${page.label}`,
        name, keywords, context, all: [name, ...keywords, ...context].join(" "),
      };
    });
}

const wordStart = (hay: string, q: string) => (" " + hay).includes(" " + q);
/** The query starts a word of `hay` — or, for a query of several words, each
 *  of them does, in any order ("server mcp" finds "Claude MCP server"). */
const starts = (hay: string, q: string, words: readonly string[]) =>
  wordStart(hay, q) || (words.length > 1 && words.every((w) => wordStart(hay, w)));

function rankOf(d: SearchDoc, q: string, words: readonly string[]): Rank | null {
  if (d.name.startsWith(q)) return "name";
  if (starts(d.name, q, words)) return "word";
  if (d.keywords.some((k) => starts(k, q, words))) return "synonym";
  if (d.context.some((c) => starts(c, q, words)) || (words.length > 1 && words.every((w) => wordStart(d.all, w)))) return "context";
  if (q.length >= SUBSTRING_MIN && d.all.includes(q)) return "substring";
  return null;
}

/** The sections `query` names, best first, at most `limit`. `include` leaves
 *  out what this window can never draw (a host-only section on a remote
 *  window); it filters BEFORE the cap, so the list is still full. An empty
 *  query — or one with no letters or digits — names nothing. */
export function searchSettings(
  index: readonly SearchDoc[],
  query: string,
  opts: { include?: (id: string) => boolean; limit?: number } = {},
): SearchHit[] {
  const q = norm(query);
  if (!q) return [];
  const words = q.split(" ");
  const include = opts.include ?? (() => true);
  const hits: { d: SearchDoc; r: number; at: number }[] = [];
  index.forEach((d, at) => {
    if (!include(d.id)) return;
    const r = rankOf(d, q, words);
    if (r) hits.push({ d, r: RANKS.indexOf(r), at });
  });
  hits.sort((a, b) => a.r - b.r || a.at - b.at);
  return hits.slice(0, opts.limit ?? SEARCH_LIMIT).map(({ d, r }) => ({
    id: d.id, page: d.page, label: d.label, where: d.where, rank: RANKS[r],
  }));
}
