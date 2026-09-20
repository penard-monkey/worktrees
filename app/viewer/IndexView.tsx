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
import { useEffect, useMemo, useRef, useState } from "react";
import { dirOf, type IndexEntry } from "./contract";

/** Case-insensitive subsequence over the haystack — "dadr" finds `docs/adr`. */
function subseq(hay: string, needle: string): boolean {
  let i = 0;
  for (const ch of hay) {
    if (ch === needle[i]) i++;
    if (i === needle.length) return true;
  }
  return i === needle.length;
}

export function matches(entry: IndexEntry, q: string): boolean {
  const n = q.trim().toLowerCase();
  if (!n) return true;
  const path = entry.path.toLowerCase();
  const title = entry.title.toLowerCase();
  return path.includes(n) || title.includes(n) || subseq(path, n) || subseq(title, n);
}

export function IndexView({
  entries,
  current,
  onOpen,
}: {
  entries: IndexEntry[];
  current: string | null;
  onOpen: (path: string) => void;
}) {
  const [q, setQ] = useState("");
  const input = useRef<HTMLInputElement | null>(null);

  // `/` focuses the filter, the one shortcut worth having on a reading surface.
  // Capture phase and an explicit target check so it never steals a keystroke
  // from the field itself.
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

  const groups = useMemo(() => {
    const hit = entries.filter((e) => matches(e, q));
    const by = new Map<string, IndexEntry[]>();
    for (const e of hit) {
      const d = dirOf(e.path) || ".";
      const list = by.get(d);
      if (list) list.push(e);
      else by.set(d, [e]);
    }
    return [...by.entries()].sort((a, b) => (a[0] === "." ? -1 : b[0] === "." ? 1 : a[0].localeCompare(b[0])));
  }, [entries, q]);

  const shown = groups.reduce((n, g) => n + g[1].length, 0);

  return (
    <div className="index">
      <div className="index-head">
        <input
          ref={input}
          className="index-filter"
          type="search"
          placeholder="filter by title or path…   (/)"
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
                  href={`#/${e.path}`}
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
