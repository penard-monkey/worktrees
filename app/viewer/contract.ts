// The wire contract, and the only place that knows its shape.
//
// Base URL: `http://127.0.0.1:<port>/<token>/p/<place>/`. Every route below is
// RELATIVE to that base, which is why no token ever appears in this bundle —
// the page was served from inside the token prefix, so `new URL("doc", base)`
// carries it. A capability in the path is exactly as strong as the page's own
// origin-plus-path, and nothing here has to be told what it is.
//
//   doc?path=<rel>   application/json, conditional (ETag / If-None-Match / 304)
//   index            application/json, the document list
//   asset/<rel>      raw bytes
//
// `410 Gone` from ANY route means the place is gone. It is handled here rather
// than at each call site so no caller can forget it: a viewer whose worktree
// was removed must say so, not poll a corpse at 1 Hz forever.

export type Meta = {
  place: string;
  branch: string;
  behind: number;
  base: string;
  dirty: number;
  subject: string;
  derived_epoch: number;
  status_epoch: number;
};

export type Block = { id: string; md: string };

export type DocPayload = { meta: Meta; blocks: Block[] };

/**
 * One row of the `index` route. See `normalizeIndex` for what is tolerated.
 *
 * `group` is the backend's OWN nesting decision and is deliberately not always
 * `path`'s directory part: `.planning/brief.md` is grouped with the ROOT files,
 * because a group of one under a gitignored directory name reads as an accident
 * (`DocEntry::group` in `docs.rs`). Deriving it from the path would file the
 * brief under a directory the walk does not show — so it is used verbatim when
 * the server sends it, and only FALLS BACK to the dirname when it does not.
 */
export type IndexEntry = { path: string; title: string; group: string };

export type IndexPayload = { meta: Meta | null; entries: IndexEntry[] };

export type Fetched<T> =
  | { kind: "same" }                          // 304 — nothing moved
  | { kind: "data"; etag: string | null; data: T }
  | { kind: "gone" }                           // 410 — the place is gone
  | { kind: "error"; message: string };

/**
 * The base every route hangs off. Taken from the document's own URL with the
 * fragment removed, because the fragment is the viewer's client-side route
 * (`#/docs/x.md`) and must not become part of a request path.
 *
 * `document.baseURI` would be the shorter spelling and is deliberately not used
 * — it is what a `<base>` element rewrites. The page's CSP carries
 * `base-uri 'none'` so one cannot take effect, but a URL helper that depends on
 * a CSP directive for its correctness is one directive away from being wrong.
 */
export function apiBase(href: string = location.href): URL {
  return new URL("./", href.split("#")[0]);
}

async function request<T>(
  url: URL,
  etag: string | null,
  parse: (body: unknown) => T,
): Promise<Fetched<T>> {
  let res: Response;
  try {
    res = await fetch(url.toString(), {
      // `no-store` because the conditional request IS the cache: a 304 we never
      // see because the HTTP cache answered it looks exactly like a server that
      // stopped updating.
      cache: "no-store",
      headers: etag ? { "If-None-Match": etag } : undefined,
    });
  } catch (e) {
    return { kind: "error", message: `cannot reach the docs server — ${String(e)}` };
  }
  if (res.status === 304) return { kind: "same" };
  if (res.status === 410) return { kind: "gone" };
  if (!res.ok) return { kind: "error", message: `${res.status} ${res.statusText} from ${url.pathname}` };
  let body: unknown;
  try {
    body = await res.json();
  } catch (e) {
    return { kind: "error", message: `${url.pathname} did not return JSON — ${String(e)}` };
  }
  try {
    return { kind: "data", etag: res.headers.get("ETag"), data: parse(body) };
  } catch (e) {
    return { kind: "error", message: `${url.pathname} — ${String(e)}` };
  }
}

const str = (v: unknown, fallback = ""): string => (typeof v === "string" ? v : fallback);
const num = (v: unknown, fallback = 0): number => (typeof v === "number" && Number.isFinite(v) ? v : fallback);

/**
 * Meta is read field by field with a fallback rather than trusted wholesale.
 * A missing `behind` that arrives as `undefined` and prints "undefined behind
 * origin/main" is the §1.1 failure with a new cause — the header's whole job is
 * to be believed, so it must never render a value it did not receive.
 * `metaPresent` below is what tells the chrome to say so out loud.
 */
function parseMeta(v: unknown): Meta | null {
  if (!v || typeof v !== "object") return null;
  const m = v as Record<string, unknown>;
  return {
    place: str(m.place, "?"),
    branch: str(m.branch, "?"),
    behind: num(m.behind, -1),
    base: str(m.base, "?"),
    dirty: num(m.dirty, -1),
    subject: str(m.subject, ""),
    derived_epoch: num(m.derived_epoch, 0),
    status_epoch: num(m.status_epoch, 0),
  };
}

function parseDoc(body: unknown): DocPayload {
  if (!body || typeof body !== "object") throw new Error("doc payload is not an object");
  const b = body as Record<string, unknown>;
  const meta = parseMeta(b.meta);
  if (!meta) throw new Error("doc payload has no `meta`");
  const raw = Array.isArray(b.blocks) ? b.blocks : null;
  if (!raw) throw new Error("doc payload has no `blocks` array");
  const blocks: Block[] = raw.map((x, i) => {
    const o = (x ?? {}) as Record<string, unknown>;
    return { id: str(o.id, `b${i}`), md: str(o.md) };
  });
  return { meta, blocks };
}

/**
 * THE CONTRACT DOES NOT SPELL THE INDEX BODY OUT — it says only
 * "application/json — the document list". Rather than pick one shape and break
 * on the server's, this accepts every shape a reasonable server would emit and
 * says which one it found. Reported as a contract gap; see the session report.
 */
export function normalizeIndex(body: unknown): IndexPayload {
  const asEntry = (x: unknown): IndexEntry | null => {
    if (typeof x === "string") return { path: x, title: basename(x), group: dirOf(x) };
    if (!x || typeof x !== "object") return null;
    const o = x as Record<string, unknown>;
    const path = str(o.path) || str(o.rel) || str(o.href) || str(o.file);
    if (!path) return null;
    // `"group" in o` rather than `str(o.group) || dirOf(path)`: a root-level
    // document's group is the empty string, and truthiness cannot tell "the
    // server said root" from "the server said nothing".
    const group = "group" in o ? str(o.group) : dirOf(path);
    return { path, title: str(o.title) || str(o.name) || basename(path), group };
  };
  const list = (v: unknown): IndexEntry[] =>
    Array.isArray(v) ? v.map(asEntry).filter((e): e is IndexEntry => e !== null) : [];

  if (Array.isArray(body)) return { meta: null, entries: list(body) };
  if (!body || typeof body !== "object") throw new Error("index payload is not an object or array");
  const b = body as Record<string, unknown>;
  const entries = list(b.entries) .concat(list(b.docs), list(b.items), list(b.files), list(b.documents));
  return { meta: parseMeta(b.meta), entries };
}

export const basename = (p: string): string => p.split("/").filter(Boolean).pop() ?? p;

export function fetchDoc(base: URL, path: string, etag: string | null): Promise<Fetched<DocPayload>> {
  const u = new URL("doc", base);
  u.searchParams.set("path", path);
  return request(u, etag, parseDoc);
}

export function fetchIndex(base: URL, etag: string | null): Promise<Fetched<IndexPayload>> {
  return request(new URL("index", base), etag, normalizeIndex);
}

/**
 * `a/b/../c.png` → `a/c.png`, and `null` if it climbs out of the place root.
 *
 * The server does its own containment check and is the authority; this one is
 * here so the page never ASKS for something outside the tree. Two boundaries
 * that agree are worth more than one, and it also means a document with a
 * traversal in it renders a visible refusal instead of a 404 image.
 */
export function resolveRel(dir: string, rel: string): string | null {
  const out: string[] = [];
  const parts = (dir ? dir.split("/") : []).concat(rel.split("/"));
  for (const p of parts) {
    if (p === "" || p === ".") continue;
    if (p === "..") {
      if (out.length === 0) return null; // climbed out
      out.pop();
      continue;
    }
    out.push(p);
  }
  return out.length ? out.join("/") : null;
}

/** `docs/adr/0001.md` → `docs/adr`; a root file → `""`. */
export const dirOf = (path: string): string => {
  const i = path.lastIndexOf("/");
  return i < 0 ? "" : path.slice(0, i);
};

export function assetUrl(base: URL, path: string): string {
  return new URL(`asset/${path.split("/").map(encodeURIComponent).join("/")}`, base).toString();
}
