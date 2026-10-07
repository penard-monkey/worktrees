// The host half of markdown path links (mdpaths.ts has the rules): asks the
// backend about a document's candidates, caches the answers for that document,
// and owns the menu a name several files share opens. FileView and PlanPane
// each hold one; anything else that renders `Markdown` passes no `pathLinks`
// and gets none.
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CtxMenu } from "./CtxMenu";
import { resolveDocPaths, type DocPathHit, type PathAnchor, type PathLinks } from "./mdpaths";

type At = { line?: number; col?: number };
const NONE: ReadonlyMap<string, DocPathHit[]> = new Map();

/**
 * `root` is the place, `doc` the markdown file's own path (its directory is
 * the first base for a relative path), `generation` the Files tree's reload
 * token: when it moves, everything on screen is asked again — a file may have
 * appeared or gone — and the old answers stay up until the new ones land, so
 * a refresh does not blink every link off and on.
 *
 * Answers are kept per (root, doc): another document starts empty.
 */
export function useDocPathLinks(
  root: string | null,
  doc: string | null,
  generation: number,
  onOpen: (path: string, at: At) => void,
): { pathLinks: PathLinks | undefined; menu: ReactNode } {
  const docKey = `${root ?? ""}\0${doc ?? ""}`;
  const [store, setStore] = useState<{ key: string; answers: ReadonlyMap<string, DocPathHit[]> }>({ key: docKey, answers: NONE });
  const answers = store.key === docKey ? store.answers : NONE;
  // What has been asked under the CURRENT (doc, generation). A ref, not state:
  // `want` runs in an effect after every render and must not cause one.
  const asked = useRef<{ key: string; set: Set<string> }>({ key: "", set: new Set() });
  // `generation` through a ref, so `want` — and with it `pathLinks` — keeps
  // its identity across a refresh: a new `pathLinks` makes Markdown rebuild
  // its whole body, and a refresh that changed nothing must not cost that.
  const genRef = useRef(generation);
  genRef.current = generation;
  const wanted = useRef<{ key: string; cands: string[] }>({ key: "", cands: [] });

  const want = useCallback((cands: string[]) => {
    if (!root) return;
    wanted.current = { key: docKey, cands };
    const generation = genRef.current;
    const askKey = `${docKey}\0${generation}`;
    if (asked.current.key !== askKey) asked.current = { key: askKey, set: new Set() };
    const seen = asked.current.set;
    const ask = cands.filter((c) => !seen.has(c));
    if (!ask.length) return;
    ask.forEach((c) => seen.add(c));
    void resolveDocPaths(invoke, { root, doc, generation }, ask).then((got) => {
      // A late answer for a document no longer shown is dropped, not merged.
      if (asked.current.key !== askKey || !got.size) return;
      setStore((prev) => {
        const old = prev.key === docKey ? prev.answers : NONE;
        // A refresh re-asks everything and usually learns nothing new; the
        // SAME store back means Markdown keeps its body instead of rebuilding.
        const same = (v: DocPathHit[], w: DocPathHit[] | undefined) =>
          !!w && v.length === w.length && v.every((h, i) => h.path === w[i].path);
        if (prev.key === docKey && [...got].every(([k, v]) => same(v, old.get(k)))) return prev;
        const m = new Map(old);
        for (const [k, v] of got) m.set(k, v);
        return { key: docKey, answers: m };
      });
    });
  }, [root, doc, docKey]);
  // A refresh: ask again about what is on screen. `want` sees the new
  // generation, so nothing counts as asked yet.
  useEffect(() => {
    const w = wanted.current;
    if (w.key === docKey && w.cands.length) want(w.cands);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- on a refresh only
  }, [generation]);

  // A ref: hosts pass an inline arrow (PlanPane's), and an `open` that changed
  // with it would hand Markdown new `pathLinks` — rebuilding the whole body,
  // ~165ms on a big doc — on every render of App.
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;
  const [menu, setMenu] = useState<{ anchor: PathAnchor; hits: DocPathHit[]; at: At } | null>(null);
  useEffect(() => setMenu(null), [docKey]);
  const open = useCallback((hits: DocPathHit[], at: At, anchor: PathAnchor) => {
    if (hits.length === 1) onOpenRef.current(hits[0].path, at);
    else setMenu({ anchor, hits, at });
  }, []);

  const pathLinks = useMemo(() => (root ? { answers, want, open } : undefined), [root, answers, want, open]);
  const node = menu && (
    <CtxMenu x={menu.anchor.x} y={menu.anchor.y} onClose={() => setMenu(null)}>
      <div className="pop-hint">open which?</div>
      {menu.hits.map((h) => (
        <button key={h.path} className="pop-item md-path-choice" title={h.path} data-track="md.path.choose"
          onClick={() => { setMenu(null); onOpenRef.current(h.path, menu.at); }}>
          {h.rel}
        </button>
      ))}
    </CtxMenu>
  );
  return { pathLinks, menu: node };
}
