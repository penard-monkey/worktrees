// What a link or an image written in a memory file names, pure: no import, so
// memory-docs-check.mjs loads this file as it is.
//
// A memory's links are plain relative markdown links (workspace-chats.md §2,
// workspace-view-design.md §4.4), resolved from the file they are in. The host
// is the authority on what may be read (`worktrees_core::memory`); this
// restates its string rule so a link that cannot be one is said to be so
// without a round trip, and so a remote image is never fetched.

export type MemoryLink =
  /** A file of this memory, if there is one: the caller knows its list. */
  | { kind: "file"; path: string; hash: string | null }
  | { kind: "idea"; id: string }
  /** A heading of the file the link is in. */
  | { kind: "anchor"; hash: string }
  | { kind: "url"; url: string }
  /** `why` is for a tooltip or a note. */
  | { kind: "none"; why: string };

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;
const IDEA = /^idea:([0-9A-HJKMNP-TV-Z]{26})$/i;

export const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];
export const isImagePath = (path: string) => IMAGE_EXTS.includes(/\.([^./]+)$/.exec(path)?.[1]?.toLowerCase() ?? "");

/** `href` as written in the memory file `from` (its path in the memory). */
export function memoryLink(from: string, href: string): MemoryLink {
  const raw = href.trim();
  if (!raw) return { kind: "none", why: "The link is empty." };
  const idea = IDEA.exec(raw);
  if (idea) return { kind: "idea", id: idea[1].toUpperCase() };
  if (/^(https?|mailto):/i.test(raw)) return { kind: "url", url: raw };
  if (SCHEME.test(raw) || raw.startsWith("//")) return { kind: "none", why: "This kind of link is not opened." };
  const at = raw.indexOf("#");
  const hash = at >= 0 ? raw.slice(at + 1) : null;
  const bare = (at >= 0 ? raw.slice(0, at) : raw).replace(/\?.*$/, "");
  if (!bare) return hash ? { kind: "anchor", hash } : { kind: "none", why: "The link is empty." };
  let path = bare;
  try { path = decodeURIComponent(bare); } catch { /* an unescaped % is part of the name */ }
  if (path.startsWith("/")) return { kind: "none", why: "Outside this memory." };
  // eslint-disable-next-line no-control-regex
  if (/[\\\u0000-\u001f\u007f]/.test(path)) return { kind: "none", why: "Not a file of this memory." };
  const out = from.split("/").slice(0, -1);
  for (const p of path.split("/")) {
    if (p === "" || p === ".") continue;
    if (p === "..") {
      if (!out.length) return { kind: "none", why: "Outside this memory." };
      out.pop();
    } else if (p.startsWith(".")) return { kind: "none", why: "Not a file of this memory." };
    else out.push(p);
  }
  return out.length ? { kind: "file", path: out.join("/"), hash } : { kind: "none", why: "Not a file of this memory." };
}

/** A link's text from the file `from` to the file `to`, the one `memoryLink`
 *  reads back: what `[[` writes. */
export function linkTo(from: string, to: string): string {
  const a = from.split("/").slice(0, -1);
  const b = to.split("/");
  let i = 0;
  while (i < a.length && i < b.length - 1 && a[i] === b[i]) i++;
  // Spaces and parentheses would end a markdown destination.
  return [...a.slice(i).map(() => ".."), ...b.slice(i)].map((p) => encodeURIComponent(p).replace(/[()]/g, (c) => (c === "(" ? "%28" : "%29"))).join("/");
}

/** A path for a text kept as a new file: `topics/<slug>.md`. */
export function topicPath(title: string, n = 1): string {
  const slug = title.trim().toLowerCase().replace(/\.(md|markdown|txt)$/, "").normalize("NFKD").replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-+|-+$/g, "").slice(0, 60);
  return `topics/${slug || "untitled"}${n > 1 ? `-${n}` : ""}.md`;
}

/** The sentences core refuses with (`memory::ERR_STALE`, `memory::ERR_GONE`). */
export const isMemoryStale = (e: string) => /changed on disk/.test(e);
export const isMemoryGone = (e: string) => /^no memory file/.test(e);
