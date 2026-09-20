// The document list, and the search over it.
//
// This is the route a static page could not have. A generated file can link to
// a sibling document only if that sibling was also generated, and a place's
// docs tree runs to hundreds of markdown files; search across them is not
// expressible at all. The brief calls this "a large part of why there is a
// server", and it is: links between documents and a filter over titles and
// paths are the whole navigation model.
//
// The filter matches BOTH title and path, because the two disagree usefully —
// `docs/adr/0007-one-engine.md` is found by "adr", by "0007" and by the words
// in its heading, and a reader arriving from a code review has the path while a
// reader arriving from a conversation has the title.
import { useMemo, useState } from "react";
import type { IndexEntry } from "./contract";
// ONE filter rule. It lives with the nav, which is where the filter now
// primarily is; this screen is the landing view and the drawer's stand-in on a
// narrow window, and a second definition of "does this row match?" would drift
// the moment either was tuned.
import { matches } from "./DocsNav";

export function IndexView({
  entries,
  current,
  onOpen,
  hrefFor,
}: {
  entries: IndexEntry[];
  current: string | null;
  onOpen: (path: string) => void;
  /** `Viewer`'s `routeHash`. The hand-written `#/${e.path}` this replaced was
   *  unescaped, and a `%` in a filename is what makes the router throw. */
  hrefFor: (path: string) => string;
}) {
  const [q, setQ] = useState("");

  // `/` IS NOT BOUND HERE. It used to be, in capture phase, and `DocsNav` bound
  // it too — on this route both are on screen, so the later registration won
  // and the nav's filter was unreachable from the keyboard. One handler now
  // lives in `Viewer` and reaches the nav's input; this screen's own box stays
  // click-to-focus.

  const groups = useMemo(() => {
    const hit = entries.filter((e) => matches(e, q));
    const by = new Map<string, IndexEntry[]>();
    for (const e of hit) {
      // `e.group`, NEVER `dirOf(e.path)`. They differ on exactly the row where
      // it matters: `.planning/brief.md` is grouped with the ROOT files by the
      // backend, and deriving the group from the path filed it under a
      // `.planning` heading here while the nav — which uses `tree()`, which
      // uses `group` — correctly put it at the root. Two surfaces on the SAME
      // PAGE disagreeing about where a document lives, which is the mirror this
      // whole extraction exists to avoid, one level smaller.
      const d = e.group || ".";
      const list = by.get(d);
      if (list) list.push(e);
      else by.set(d, [e]);
    }
    // NOT SORTED. The nav beside this on the very same route deliberately
    // refuses to sort — `docs::index_with` puts the root files in a fixed
    // reading order and `[docs] paths = ["b", "a"]` is listed b-then-a because
    // the repo said so (`app/scripts/docs-check.mjs` exists to keep that true).
    // A `localeCompare` here overruled the project's own choice on one of the
    // two surfaces, so the index and the tree showed the same documents in
    // DIFFERENT orders, side by side, and nothing failed. Insertion order is
    // the server's order, which is the answer both surfaces must give.
    return [...by.entries()];
  }, [entries, q]);

  const shown = groups.reduce((n, g) => n + g[1].length, 0);

  return (
    <div className="index">
      <div className="index-head">
        <input
          className="index-filter"
          type="search"
          // No `(/)` any more: the shortcut reaches the NAV's filter, and two
          // boxes advertising the same key is how the collision read on screen.
          placeholder="filter by title or path…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
        <span className="index-count" data-shown={shown} data-total={entries.length}>
          {shown === entries.length ? `${entries.length} documents` : `${shown} of ${entries.length}`}
        </span>
      </div>
      {entries.length === 0 && <div className="index-empty">this place has no documents</div>}
      {entries.length > 0 && shown === 0 && <div className="index-empty">nothing matches “{q}”</div>}
      {groups.map(([dir, rows]) => (
        <section className="index-group" key={dir}>
          <h2 className="index-dir">{dir === "." ? "(root)" : dir}</h2>
          <ul className="index-list">
            {rows.map((e) => (
              <li key={e.path}>
                <a
                  className={`index-row${e.path === current ? " is-current" : ""}`}
                  href={hrefFor(e.path)}
                  onClick={(ev) => { ev.preventDefault(); onOpen(e.path); }}
                >
                  <span className="index-title">{e.title}</span>
                  <span className="index-path">{e.path}</span>
                </a>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
