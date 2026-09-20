// The persistent document navigation.
//
// This is the thing a generated page could not have, and the reason the
// `docs-render` investigation called navigation the weakness of a static tree:
// "the thing a docs site is actually for — following a link from one page to
// the next." Until now the document list was a SCREEN you navigated to and away
// from, so the tree and a document were never visible at once.
//
// IT REUSES `app/src/doctree.ts::tree`, the same function the app's Docs tab
// renders. Not a copy — the ORDER is load-bearing and is not the obvious one:
// `docs::index_with` puts the root files in a fixed reading order, and
// `[docs] paths = ["b", "a"]` is listed b-then-a because the repo said so.
// Sorting here would overrule the project's own choice and nothing would fail.
// `app/scripts/docs-check.mjs` slices the real function and is what keeps that
// true; it was re-pointed at `doctree.ts` when the function moved.
//
// The ROW rendering is this file's own, and that is the right seam: the dock's
// rows carry a modified-since-you-looked dot, an mtime, a "browse" action and a
// busy state, none of which exist here. What must not be duplicated is the
// rule, and the rule is `tree()`.
import { useEffect, useMemo, useRef, useState } from "react";
import { tree, type DocEntry, type DocNode } from "../src/doctree";
import type { IndexEntry } from "./contract";

/** Case-insensitive subsequence — "dadr" finds `docs/adr`. */
function subseq(hay: string, needle: string): boolean {
  let i = 0;
  for (const ch of hay) {
    if (ch === needle[i]) i++;
    if (i === needle.length) return true;
  }
  return i === needle.length;
}

/** Matches BOTH title and path, because the two disagree usefully: a reader
 *  arriving from a code review has the path, one arriving from a conversation
 *  has the title. */
export function matches(entry: IndexEntry, q: string): boolean {
  const n = q.trim().toLowerCase();
  if (!n) return true;
  const path = entry.path.toLowerCase();
  const title = entry.title.toLowerCase();
  return path.includes(n) || title.includes(n) || subseq(path, n) || subseq(title, n);
}

/** `IndexEntry` → the shape `tree()` consumes. `mtime_ms` is 0 because the
 *  wire does not carry it and nothing here draws the dock's freshness dot. */
const asDocEntry = (e: IndexEntry): DocEntry =>
  ({ path: e.path, rel: e.path, title: e.title, group: e.group, mtime_ms: 0 });

/**
 * Collapse state, per place.
 *
 * Keyed by the PLACE and never by the token — the token is regenerated every
 * launch, so a key carrying it would forget on every restart. Worth knowing
 * that this does not survive a relaunch anyway, for a reason further down the
 * stack: the port is part of the origin, the port is ephemeral per launch, and
 * `localStorage` is scoped to the origin. So this survives a RELOAD and a
 * navigation, which are the cases that matter, and a relaunch starts fresh.
 *
 * Every read and write is wrapped: storage throws in a private window, with
 * site data blocked, and during thumbnail capture.
 */
function useCollapsed(place: string): [Set<string>, (path: string) => void] {
  const key = `worktrees.docsnav.collapsed.${place}`;
  const [closed, setClosed] = useState<Set<string>>(() => {
    try {
      const raw = localStorage.getItem(key);
      return new Set(raw ? (JSON.parse(raw) as string[]) : []);
    } catch {
      return new Set();
    }
  });
  const toggle = (path: string) =>
    setClosed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      try { localStorage.setItem(key, JSON.stringify([...next])); } catch { /* not worth a message */ }
      return next;
    });
  return [closed, toggle];
}

function Rows(p: {
  nodes: DocNode[];
  depth: number;
  closed: ReadonlySet<string>;
  /** A filter is active, so children show whatever `closed` says. A match
   *  hidden inside a collapsed directory is a filter that looks broken. */
  filtering: boolean;
  current: string | null;
  onToggle: (path: string) => void;
  onOpen: (path: string) => void;
}) {
  return (
    <>
      {p.nodes.map((n) => {
        if (n.kind === "dir") {
          const open = !p.closed.has(n.path);
          // The chevron follows what the user CHOSE; the children follow what
          // the filter NEEDS. Same split as the dock's rows, and for the same
          // reason: a chevron that ignores a click is a control that looks
          // broken, so the filtered view shows a closed directory's matches
          // rather than silently changing what the chevron means.
          const show = open || p.filtering;
          return (
            <div className="dnav-branch" key={`dir:${n.path}`}>
              <button
                className="dnav-dir"
                style={{ ["--depth" as string]: p.depth }}
                aria-expanded={open}
                title={n.path}
                onClick={() => p.onToggle(n.path)}
              >
                <span className="dnav-chev" aria-hidden>{open ? "▾" : "▸"}</span>
                <span className="dnav-dirname">{n.name}</span>
                <span className="dnav-count">{n.count}</span>
              </button>
              {show && <Rows {...p} nodes={n.kids} depth={p.depth + 1} />}
            </div>
          );
        }
        const e = n.entry;
        const isCurrent = p.current === e.rel;
        return (
          <a
            key={e.path}
            className={`dnav-row${isCurrent ? " is-current" : ""}`}
            style={{ ["--depth" as string]: p.depth }}
            href={`#/${e.rel}`}
            title={e.rel}
            // `aria-current` rather than only a class: the mark has to be the
            // document's state, not just a colour.
            aria-current={isCurrent ? "page" : undefined}
            onClick={(ev) => { ev.preventDefault(); p.onOpen(e.rel); }}
          >
            <span className="dnav-title">{e.title}</span>
          </a>
        );
      })}
    </>
  );
}

export function DocsNav({
  entries, place, current, onOpen,
}: {
  entries: IndexEntry[];
  place: string;
  current: string | null;
  onOpen: (path: string) => void;
}) {
  const [q, setQ] = useState("");
  const [closed, toggle] = useCollapsed(place);
  const input = useRef<HTMLInputElement | null>(null);

  // `/` focuses the filter — one shortcut, on a reading surface. Capture phase
  // with an explicit target check so it never steals a keystroke from a field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "/" || e.metaKey || e.ctrlKey || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      e.preventDefault();
      input.current?.focus();
      input.current?.select();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const filtering = q.trim() !== "";
  // Filter the ENTRIES and then build the tree from the survivors, so a
  // directory with no matches does not appear at all. `filtering` above then
  // forces the survivors open, which is what "auto-expand to reveal matches"
  // means — two mechanisms, because either alone leaves a case: filtering only
  // would leave a collapsed parent hiding its matches, and force-opening only
  // would leave empty directories in the way.
  const nodes = useMemo(
    () => tree(entries.filter((e) => matches(e, q)).map(asDocEntry)),
    [entries, q],
  );
  const shown = entries.filter((e) => matches(e, q)).length;

  return (
    <nav className="dnav" aria-label="documents in this place">
      <div className="dnav-head">
        <input
          ref={input}
          className="dnav-filter"
          type="search"
          placeholder="filter…  (/)"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
        <span className="dnav-total" data-shown={shown} data-total={entries.length}>
          {shown === entries.length ? entries.length : `${shown}/${entries.length}`}
        </span>
      </div>
      <div className="dnav-tree">
        {entries.length === 0 && <div className="dnav-empty">no documents</div>}
        {entries.length > 0 && shown === 0 && <div className="dnav-empty">nothing matches “{q}”</div>}
        <Rows nodes={nodes} depth={0} closed={closed} filtering={filtering} current={current} onToggle={toggle} onOpen={onOpen} />
      </div>
    </nav>
  );
}
