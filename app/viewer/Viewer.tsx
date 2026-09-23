// The viewer root: routing, the poll loop, and the three screens.
//
// ROUTING lives entirely in the fragment. The server serves one shell for
// `/<token>/p/<place>/` and knows nothing about which document is open, so
// `#/docs/adr/0007.md` needs no server cooperation, gives a real back button,
// and produces a URL that can be handed to a colleague. An in-document heading
// rides along as `?h=<slug>` INSIDE the fragment rather than as a second `#`,
// which a URL cannot express — so a link to a section is shareable too.
//
// THE POLL is `doc` with `If-None-Match` at 1 Hz. docs-transport §3.1 measured
// why it is not SSE: a browser allows six connections per origin, a held
// EventSource spends one, and the SIXTH open tab starves every other request on
// that origin — images stop loading, navigation hangs. That is a cliff, not
// degradation, and it arrives as "the docs viewer is broken". A conditional GET
// costs 410 bytes and 0.149 ms against 11 018 bytes for the document, holds no
// connection, and puts the change signal in HTTP rather than in machinery we
// own. On 304 this code writes no DOCUMENT state — so React does not render,
// so no DOM node is touched. (It may clear a stale error banner, through a ref
// that makes "already null" cost nothing; a 304 after an outage is the only
// signal that the server came back, and a banner that outlives the outage is a
// worse lie than the outage.) What the 304 does NOT do is supply the text, so
// the payload of every document visited is cached per path beside its ETag —
// see `DOC_CACHE_MAX`.
//
// A CHAINED setTimeout, never setInterval: a server that takes longer than the
// interval must not accumulate a queue of overlapping requests.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Chrome } from "./Chrome";
import { DocBody, type DocCtx } from "./blocks";
import { DocsNav } from "./DocsNav";
import { IndexView } from "./IndexView";
import {
  apiBase, basename, fetchDoc, fetchIndex,
  type DocPayload, type IndexPayload, type Meta,
} from "./contract";
import { useDocView, ZOOM_DEFAULT, ZOOM_MAX, ZOOM_MIN, type DocViewApi } from "./zoom";

const POLL_MS = 1000;

/**
 * How many documents' payloads are kept so that going BACK to one is instant.
 *
 * Bounded because a place can hold hundreds of documents and a payload is the
 * whole text (11 KB for a typical one, measured in docs-transport §3.1) — a map
 * that only grows would hold a place's entire docs tree in memory for a reader
 * who wandered through it. 24 is far more than the handful a reader moves
 * between and small enough to be uninteresting (~0.3 MB at that size). Eviction
 * is by least-recently-updated, and it drops the ETag with the body.
 */
const DOC_CACHE_MAX = 24;

/** One cached document: the body AND the ETag that describes it, together. */
type DocEntryCache = { etag: string | null; payload: DocPayload };

export type Route = { kind: "index" } | { kind: "doc"; path: string; anchor: string | null };

/**
 * `#/<path>` → a document; anything else → the index.
 *
 * THE WHOLE BODY IS IN A `try`, because `decodeURIComponent` throws `URIError`
 * on a stray `%` — and a URL with one is not exotic: a document named `100%.md`
 * produces `#/100%.md` the moment anything writes an unescaped href (which
 * `DocsNav` did). This function is called from a `useState` INITIALISER, so a
 * throw there is a render-time throw with no error boundary above it: a blank
 * page. It is also called from the `hashchange` listener, where a throw freezes
 * the route at whatever was last parsed — the viewer simply stops navigating,
 * silently. Falling back to the index is the one behaviour that is visibly
 * wrong rather than invisibly broken.
 */
export function parseRoute(hash: string): Route {
  try {
    const h = hash.startsWith("#") ? hash.slice(1) : hash;
    if (!h.startsWith("/")) return { kind: "index" };
    const raw = h.slice(1);
    if (!raw) return { kind: "index" };
    const q = raw.indexOf("?h=");
    if (q < 0) return { kind: "doc", path: decodeURIComponent(raw), anchor: null };
    return {
      kind: "doc",
      path: decodeURIComponent(raw.slice(0, q)),
      anchor: decodeURIComponent(raw.slice(q + 3)) || null,
    };
  } catch {
    return { kind: "index" };
  }
}

export function routeHash(route: Route): string {
  if (route.kind === "index") return "#";
  const p = route.path.split("/").map(encodeURIComponent).join("/");
  return route.anchor ? `#/${p}?h=${encodeURIComponent(route.anchor)}` : `#/${p}`;
}

function useHashRoute(): [Route, (r: Route) => void] {
  const [route, setRoute] = useState<Route>(() => parseRoute(location.hash));
  useEffect(() => {
    const on = () => setRoute(parseRoute(location.hash));
    window.addEventListener("hashchange", on);
    return () => window.removeEventListener("hashchange", on);
  }, []);
  const go = useCallback((r: Route) => {
    const next = routeHash(r);
    if (next === (location.hash || "#")) setRoute(r);
    else location.hash = next; // the hashchange listener above picks it up
  }, []);
  return [route, go];
}

/**
 * The reading-size stepper and the measure toggle, in the sticky header.
 *
 * MODULE SCOPE, not a closure inside `Viewer` — a component defined inside
 * another re-mounts on every render of its parent (new identity), and this one
 * lives in a header that re-renders once a second as the staleness ages count
 * up (CLAUDE.md, Tauri app rules).
 *
 * The classes are App.css's own (`.seg` / `.seg-b` / `.zoomseg` / `.zoom-val`,
 * `.ctrl.sm`), which this bundle already loads whole. That is reuse of the SAME
 * control, not a borrowed name: the dock's Files pane paints A− | 100% | A+
 * from these rules for exactly this job, and the two steppers should not look
 * like different features.
 */
function ViewControls({ view, nudge, setWide }: DocViewApi) {
  const { zoom, wide } = view;
  return (
    <span className="chrome-view">
      <span className="seg zoomseg" role="group" aria-label="Reading size">
        <button
          className="seg-b"
          disabled={zoom <= ZOOM_MIN}
          title="Smaller text (⌘−)"
          aria-label="Smaller text"
          onClick={() => nudge(-1)}
        >A−</button>
        {/* The readout IS the reset. A percentage you cannot click back to 100%
            leaves the only way home as counting steps — the same decision, and
            the same control, as the Files pane's stepper. */}
        <button
          className="seg-b zoom-val"
          disabled={zoom === ZOOM_DEFAULT}
          title="Reset reading size (⌘0)"
          aria-label={`Reading size ${zoom}%. Reset to 100%.`}
          onClick={() => nudge(0)}
        >{zoom}%</button>
        <button
          className="seg-b"
          disabled={zoom >= ZOOM_MAX}
          title="Larger text (⌘+)"
          aria-label="Larger text"
          onClick={() => nudge(1)}
        >A+</button>
      </span>
      <button
        className={"ctrl sm" + (wide ? " on" : "")}
        aria-pressed={wide}
        title={
          wide
            ? "Back to a 78-character reading measure"
            : "Let the prose run the full width of the window"
        }
        onClick={() => setWide(!wide)}
      >Wide</button>
    </span>
  );
}

export function Viewer() {
  const base = useMemo(() => apiBase(), []);
  const [route, go] = useHashRoute();
  const [doc, setDoc] = useState<{ path: string; payload: DocPayload } | null>(null);
  const [index, setIndex] = useState<IndexPayload | null>(null);
  const [gone, setGone] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [updates, setUpdates] = useState(0);
  // The nav is a DRAWER below NARROW_PX and a column above it. Open-ness only
  // means anything in drawer mode; above the breakpoint the column is always
  // there and this flag is ignored.
  const [drawer, setDrawer] = useState(false);
  // The reading size and the measure. Bound to ⌘+ / ⌘− / ⌘0 only on the
  // DOCUMENT route: the handler calls `preventDefault` to keep the browser from
  // zooming the whole page underneath it, and swallowing the chord on a screen
  // where it does nothing would be worse than not binding it.
  const docView = useDocView(route.kind === "doc");
  const etags = useRef(new Map<string, string | null>());
  const docs = useRef(new Map<string, DocEntryCache>());
  const navFilter = useRef<HTMLInputElement | null>(null);

  const path = route.kind === "doc" ? route.path : null;

  // The banner, written through a ref so that "nothing changed" really writes
  // nothing. A bare `setErr(null)` on every 304 would re-render the whole page
  // once a second, which is the one thing the conditional GET exists to avoid.
  const errRef = useRef<string | null>(null);
  const showErr = useCallback((m: string | null) => {
    if (errRef.current === m) return;
    errRef.current = m;
    setErr(m);
  }, []);

  // ── the doc poll ────────────────────────────────────────────────────────
  //
  // THE ETAG AND THE PAYLOAD ARE ONE ENTRY, and that is the fix for the bug
  // this loop shipped with. They used to be separate: the ETag was remembered
  // per path forever and the document was a single slot written only on a 200.
  // So A → B → A sent A's `If-None-Match`, the server correctly answered 304,
  // the 304 branch deliberately does nothing, and the slot still held B — the
  // page said `loading docs/a.md…` at 1 Hz until some file in the place
  // happened to change. Back button, `#/` deep link and a nav click all hit it,
  // and `cache: "no-store"` means the browser cache cannot paper over it.
  //
  // Keeping them together makes the 304 mean what it says ("what you have is
  // current") and makes eviction safe: drop the entry and the next request is
  // unconditional, because the ETag left with the body it described. An entry
  // that kept its ETag after losing its payload would be the same bug again,
  // permanently, for that path.
  useEffect(() => {
    if (gone || path === null) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      const r = await fetchDoc(base, path, docs.current.get(path)?.etag ?? null);
      if (stop) return;
      if (r.kind === "gone") { setGone(true); return; }
      if (r.kind === "error") showErr(r.message);
      // A 304 writes no document state — not even a setState, because a state
      // write with an equal value still re-renders once. It DOES clear the
      // banner: a transient failure followed by a 304 is a server that came
      // back, and a red "cannot reach the docs server" that outlives the
      // outage is a worse lie than the outage was.
      if (r.kind === "same") showErr(null);
      if (r.kind === "data") {
        const c = docs.current;
        // Delete-then-set so the insertion order is recency: `Map` keeps the
        // original position on a plain overwrite, which would evict the
        // document being read.
        c.delete(path);
        c.set(path, { etag: r.etag, payload: r.data });
        while (c.size > DOC_CACHE_MAX) {
          const oldest = c.keys().next();
          if (oldest.done) break;
          c.delete(oldest.value);
        }
        showErr(null);
        setDoc({ path, payload: r.data });
        setUpdates((n) => n + 1);
      }
      if (!stop) timer = setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => { stop = true; if (timer) clearTimeout(timer); };
  }, [base, path, gone, showErr]);

  // ── the index poll ──────────────────────────────────────────────────────
  //
  // Polled on BOTH routes, at the same 1 Hz as the document. It used to be
  // fetched once, lazily, while a document was open, because it was a screen
  // you had left; it is now the navigation and is on screen the whole time, so
  // a document added to the place has to appear in the list while you are
  // looking at the list. (A `const once = false` and a `!once` survived that
  // change as dead code, together with a comment describing the behaviour it
  // had switched off — removed, because a reader has no way to tell a disabled
  // knob from one that is about to be turned back on.)
  useEffect(() => {
    if (gone) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      const r = await fetchIndex(base, etags.current.get("index") ?? null);
      if (stop) return;
      if (r.kind === "gone") { setGone(true); return; }
      if (r.kind === "data") {
        etags.current.set("index", r.etag);
        setIndex(r.data);
      }
      // The banner is owned by the ROUTE's own poll, which is why both writes
      // are guarded: on a document route the doc poll is the only writer, and
      // on the index route this one is. Two unguarded writers would fight once
      // a second over which failure the reader is being shown. And `same`
      // clears as well as `data`, because a recovered index IS a 304 — without
      // that line the red banner outlives every outage it describes, on the one
      // route where nothing else ever clears it.
      if (route.kind === "index") {
        if (r.kind === "error") showErr(r.message);
        if (r.kind === "data" || r.kind === "same") showErr(null);
      }
      if (!stop) timer = setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => { stop = true; if (timer) clearTimeout(timer); };
  }, [base, route.kind, gone, showErr]);

  // ── what is on screen ───────────────────────────────────────────────────
  //
  // READ FROM THE CACHE DURING RENDER, never from a "current document" slot.
  // That is what makes going back instant AND correct: the route changes, this
  // render already finds the payload, and the conditional GET that follows a
  // beat later merely confirms it with a 304. A slot filled by an effect would
  // paint one frame of `loading …` on every revisit even with the cache in
  // place — and would go back to painting it forever the moment a 304 arrived
  // first, which is exactly the bug. `docs` is a ref, so this read is a plain
  // `Map.get` with no extra render; `doc` state below is what schedules the
  // render when a poll brings something new.
  const payload: DocPayload | null = path === null ? null : docs.current.get(path)?.payload ?? null;

  // ── meta ────────────────────────────────────────────────────────────────
  // The document's meta is preferred: it is the one that arrived with the text
  // on screen. The index's is the fallback for the index screen itself.
  const meta: Meta | null = payload?.meta ?? index?.meta ?? doc?.payload.meta ?? null;

  useEffect(() => {
    // `<place> · <doc>`, place FIRST — a tab strip truncates from the right, so
    // `README.md — ssdlc` and `README.md — messaging` truncate to the same
    // string, and the place is the discriminating half (docs-transport §5).
    const who = meta?.place ?? "docs";
    document.title = route.kind === "doc" ? `${who} · ${basename(route.path)}` : `${who} · documents`;
  }, [meta?.place, route]);

  // Probe surface: attributes, not a global. A harness can read these without
  // this bundle exporting anything, and they cost one attribute write.
  useEffect(() => {
    const r = document.documentElement;
    r.dataset.route = route.kind === "doc" ? route.path : "index";
    r.dataset.updates = String(updates);
    r.dataset.gone = gone ? "1" : "0";
  }, [route, updates, gone]);

  // ── anchors ─────────────────────────────────────────────────────────────
  // Scrolls ONLY when the requested anchor changes. Scrolling on every poll
  // would undo the very thing block patching is for.
  const scrolled = useRef<string | null>(null);
  useEffect(() => {
    if (route.kind !== "doc" || !payload) return;
    const want = `${route.path}#${route.anchor ?? ""}`;
    if (scrolled.current === want) return;
    scrolled.current = want;
    if (!route.anchor) { window.scrollTo(0, 0); return; }
    // `block: "start"` lands the heading at the top of the SCROLLPORT, which
    // is underneath the sticky `.chrome`. `scroll-margin-top` on the target is
    // what the browser subtracts here and for `:target`-style navigation alike
    // — see `.doc [id]` in viewer.css, which resolves it against the header's
    // measured height rather than a guess.
    document.getElementById(route.anchor)?.scrollIntoView({ block: "start" });
  }, [route, payload]);

  // ── `/` focuses the filter ──────────────────────────────────────────────
  //
  // ONE registration, here, because there were two: `DocsNav` and `IndexView`
  // each installed a capture-phase `/` handler with its own filter state, and
  // on the index route both are mounted. The later registration wins a capture
  // listener that calls `preventDefault`, so the NAV's filter — the one that is
  // on screen on every route — was unreachable from the keyboard, silently.
  //
  // It opens the drawer first: below the breakpoint the nav is translated
  // off-canvas rather than hidden, so focusing its input without opening it
  // would put the caret in a box the reader cannot see. Above the breakpoint
  // the flag is ignored, so this costs nothing there.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "/" || e.metaKey || e.ctrlKey || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      e.preventDefault();
      setDrawer(true);
      navFilter.current?.focus();
      navFilter.current?.select();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const ctx: DocCtx = useMemo(
    () => ({
      base,
      dir: path === null ? "" : path.replace(/[^/]*$/, "").replace(/\/$/, ""),
      onDoc: (p: string) => {
        const [pp, a] = p.split("#");
        go({ kind: "doc", path: pp, anchor: a || null });
      },
      onAnchor: (slug: string) => {
        if (path === null) return;
        go({ kind: "doc", path, anchor: slug });
      },
    }),
    [base, path, go],
  );

  if (gone) {
    return (
      <div className="gone" data-gone="1">
        <h1>This place is gone.</h1>
        <p>
          The server answered <code>410 Gone</code>. The worktree these documents came from has been removed, so
          there is nothing left to read and nothing left to poll.
        </p>
        <p className="gone-note">
          Nothing is left on screen to read: this replaces the whole page, because the document it was showing
          belonged to a worktree that no longer exists. Close the tab, or reopen the documents from a place that
          is still there.
        </p>
      </div>
    );
  }

  // `← all documents` is GONE, deliberately. It was a way back to a screen, and
  // the screen is now a column that never leaves — a link to the place you are
  // already looking at. What replaces it is the drawer toggle, which is the
  // only thing that link still meant on a narrow window.
  const nav = (
    <>
      <button
        className="nav-toggle"
        aria-expanded={drawer}
        onClick={() => setDrawer((d) => !d)}
        title="show or hide the document list"
      >
        ☰ documents
      </button>
      <span className="nav-path">{route.kind === "doc" ? route.path : "documents in this place"}</span>
      {route.kind === "doc" && <ViewControls {...docView} />}
    </>
  );

  const open = (p: string) => { setDrawer(false); go({ kind: "doc", path: p, anchor: null }); };
  const docHref = (p: string) => routeHash({ kind: "doc", path: p, anchor: null });

  return (
    <div className="shell" data-drawer={drawer ? "open" : "closed"}>
      {index && (
        <DocsNav
          entries={index.entries}
          place={meta?.place ?? "unknown"}
          current={route.kind === "doc" ? route.path : null}
          onOpen={open}
          // Both surfaces render a REAL href (middle-click, copy-link, the
          // status bar), and both wrote it by hand as `#/${rel}` — unescaped.
          // A document called `100%.md` then produced `#/100%.md`, which is the
          // input that made `decodeURIComponent` throw. `routeHash` is the one
          // function that names a document in a URL; it is passed down rather
          // than imported to keep the module graph acyclic (this file already
          // imports both components).
          hrefFor={docHref}
          filterRef={navFilter}
        />
      )}
      {/* Closes the drawer by clicking beside it. Only ever hit-testable in
          drawer mode — above the breakpoint it is `display: none`, so it can
          never sit invisibly over the prose. */}
      <div className="dnav-scrim" onClick={() => setDrawer(false)} aria-hidden />
      <div className="page">
        <Chrome meta={meta} nav={nav} />
        <main className="body">
          {err && <div className="err" role="alert">{err}</div>}
          {route.kind === "index" &&
            (index ? (
              <IndexView entries={index.entries} current={null} onOpen={open} hrefFor={docHref} />
            ) : (
              <div className="loading">loading the document list…</div>
            ))}
          {route.kind === "doc" &&
            (payload ? (
              <DocBody blocks={payload.blocks} ctx={ctx} view={docView.view} />
            ) : (
              <div className="loading">loading {route.path}…</div>
            ))}
        </main>
      </div>
    </div>
  );
}
