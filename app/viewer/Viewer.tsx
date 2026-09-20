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
// own. On 304 this code does nothing at all — literally nothing: no state is
// written, so React does not render, so no DOM node is touched.
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

const POLL_MS = 1000;

export type Route = { kind: "index" } | { kind: "doc"; path: string; anchor: string | null };

export function parseRoute(hash: string): Route {
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
  const etags = useRef(new Map<string, string | null>());

  const path = route.kind === "doc" ? route.path : null;

  // ── the doc poll ────────────────────────────────────────────────────────
  useEffect(() => {
    if (gone || path === null) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const key = `doc:${path}`;
    const tick = async () => {
      const r = await fetchDoc(base, path, etags.current.get(key) ?? null);
      if (stop) return;
      if (r.kind === "gone") { setGone(true); return; }
      if (r.kind === "error") setErr(r.message);
      // A 304 is the common case and it does nothing — not even a setState,
      // because a state write with an equal value still re-renders once.
      if (r.kind === "data") {
        etags.current.set(key, r.etag);
        setErr(null);
        setDoc({ path, payload: r.data });
        setUpdates((n) => n + 1);
      }
      if (!stop) timer = setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => { stop = true; if (timer) clearTimeout(timer); };
  }, [base, path, gone]);

  // ── the index poll ──────────────────────────────────────────────────────
  // Polled while it is on screen; fetched once, lazily, when a document is open
  // (the chrome's "documents" link needs it to exist, not to be fresh).
  useEffect(() => {
    if (gone) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Previously the index was fetched once, lazily, while a document was open,
    // because it was a screen you had left. It is now the navigation and is on
    // screen the whole time, so it is polled on both routes — a document added
    // to the place has to appear in the list while you are looking at the list.
    const once = false;
    const tick = async () => {
      const r = await fetchIndex(base, etags.current.get("index") ?? null);
      if (stop) return;
      if (r.kind === "gone") { setGone(true); return; }
      if (r.kind === "data") {
        etags.current.set("index", r.etag);
        setIndex(r.data);
      }
      if (r.kind === "error" && route.kind === "index") setErr(r.message);
      if (!stop && !once) timer = setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => { stop = true; if (timer) clearTimeout(timer); };
  }, [base, route.kind, gone]);

  // ── meta ────────────────────────────────────────────────────────────────
  // The document's meta is preferred: it is the one that arrived with the text
  // on screen. The index's is the fallback for the index screen itself.
  const meta: Meta | null =
    (route.kind === "doc" && doc?.path === path ? doc.payload.meta : null) ?? index?.meta ?? doc?.payload.meta ?? null;

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
    if (route.kind !== "doc" || doc?.path !== route.path) return;
    const want = `${route.path}#${route.anchor ?? ""}`;
    if (scrolled.current === want) return;
    scrolled.current = want;
    if (!route.anchor) { window.scrollTo(0, 0); return; }
    document.getElementById(route.anchor)?.scrollIntoView({ block: "start" });
  }, [route, doc]);

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
          Whatever is on screen behind this message is the last state that existed. It is not being updated.
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
    </>
  );

  const open = (p: string) => { setDrawer(false); go({ kind: "doc", path: p, anchor: null }); };

  return (
    <div className="shell" data-drawer={drawer ? "open" : "closed"}>
      {index && (
        <DocsNav
          entries={index.entries}
          place={meta?.place ?? "unknown"}
          current={route.kind === "doc" ? route.path : null}
          onOpen={open}
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
              <IndexView entries={index.entries} current={null} onOpen={open} />
            ) : (
              <div className="loading">loading the document list…</div>
            ))}
          {route.kind === "doc" &&
            (doc?.path === route.path ? (
              <DocBody blocks={doc.payload.blocks} ctx={ctx} />
            ) : (
              <div className="loading">loading {route.path}…</div>
            ))}
        </main>
      </div>
    </div>
  );
}
