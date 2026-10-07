// File paths in RENDERED MARKDOWN → links that open the file in the Files tab.
// The terminal's ⌘-click links (termlinks.ts) met again in prose, with the same
// division of labour: this module only decides what is worth ASKING about, and
// the backend (`resolve_doc_paths`, which calls the terminal's own
// `resolve_term_path`) decides what is a file. So a candidate costs a stat,
// never a guess, and a token that names nothing renders exactly as before.
//
// There is no separate walk over the token tree to find candidates: the
// renderer in markdown.tsx calls `codeSpanHit` / `textHits` at the leaves it
// draws and records what it asked, so the set asked about and the set drawn as
// links cannot drift apart. What is NOT mined follows from where the renderer
// does not call these: fenced code, raw HTML, and the text of an authored
// `[link](…)`.
//
// `mdpaths-check.mjs` pins the candidate rules, the batching and the wiring.
import { findPaths, type PathHit } from "./termlinks";

/** One file a candidate names — `lib.rs`'s `DocPathHit`. */
export type DocPathHit = { path: string; rel: string };

/** Where a link was clicked: the menu for an ambiguous name opens there. */
export type PathAnchor = { x: number; y: number };

/** What `Markdown` needs from its host to draw path links. */
export type PathLinks = {
  /** candidate as written → the files it names. Absent = not answered (yet),
   *  empty = names nothing; both render as plain text. */
  answers: ReadonlyMap<string, DocPathHit[]>;
  /** every candidate in the document as rendered, after each render */
  want: (candidates: string[]) => void;
  /** a click on a candidate that names at least one file */
  open: (hits: DocPathHit[], at: { line?: number; col?: number }, anchor: PathAnchor) => void;
};

/** `lib.rs`'s `TERM_PATHS_MAX`: the most one `resolve_doc_paths` call reads.
 *  A document holds more than a terminal row, so it is asked in batches —
 *  candidates past this are batched, never dropped. */
export const DOC_PATHS_BATCH = 64;

/** An inline code span is a candidate only when it IS a path from end to end
 *  (an optional `:line[:col]` included): `app/src/App.tsx:42`, `quota.rs`,
 *  `.gitignore` — but not `npm run build`, `foo.bar()` or `x.ts, y.ts`. A
 *  bare name is fine here: backticks are the author saying "this is a name". */
export function codeSpanHit(text: string): PathHit | null {
  const t = text.trim();
  if (!t || t.length > 1024) return null;
  const hits = findPaths(t);
  if (hits.length !== 1) return null;
  const h = hits[0];
  if (h.start !== 0) return null;
  // `:42-50` — findPaths takes the 42 and leaves the range end behind.
  return h.end === t.length || /^-\d+$/.test(t.slice(h.end)) ? h : null;
}

/** Plain prose: only a token with a `/` in it. `App.tsx` in a sentence is a
 *  word until somebody puts it in backticks — a stat per capitalised noun with
 *  a dot is noise, and so would be the links. */
export function textHits(text: string): PathHit[] {
  return findPaths(text).filter((h) => h.path.includes("/"));
}

/** `items` in runs of at most `n`. */
export function batches<T>(items: T[], n = DOC_PATHS_BATCH): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += n) out.push(items.slice(i, i + n));
  return out;
}

type Invoke = <R>(cmd: string, args: Record<string, unknown>) => Promise<R>;

/** Every candidate's answer, one `resolve_doc_paths` call per batch. A batch
 *  that fails is logged and left unanswered (plain text) — the rest still
 *  land. `invoke` is a parameter so the check can count the calls. */
export async function resolveDocPaths(
  invoke: Invoke,
  req: { root: string; doc: string | null; generation: number },
  paths: string[],
): Promise<Map<string, DocPathHit[]>> {
  const out = new Map<string, DocPathHit[]>();
  for (const batch of batches([...new Set(paths)])) {
    try {
      const got = await invoke<DocPathHit[][]>("resolve_doc_paths", { ...req, paths: batch });
      batch.forEach((p, i) => out.set(p, got[i] ?? []));
    } catch (e) {
      invoke("log_event", { level: "warn", msg: `markdown path links: ${e}` }).catch(() => {});
    }
  }
  return out;
}
