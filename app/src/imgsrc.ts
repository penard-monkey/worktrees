// What the CHAT does with `![alt](src)` (feat/chat-images). Pure, so
// `chatimage-check.mjs` runs this file's real source.
//
// The rule is the host's (`safepath::place_image`), repeated here ONLY to say
// the refusal without a round trip and to keep a remote URL from ever reaching
// an `<img>`; the host is the authority and judges every path again.
//   - `http(s)`, protocol-relative, `data:` and every other scheme: a note,
//     never fetched (the Files pane's rule, now the chat's).
//   - relative: resolved against the PLACE's root, which is what the host does
//     with it, so it goes through as written;
//   - absolute: only under the place's root, and sent as written;
//   - `..`, a path outside the root, or a file that is no image: a note that
//     names the path. Never a broken image and never a silent drop.

/** Extensions the chat shows: the host's `safepath::IMAGE_TYPES`. */
export const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

export type ImageSrc =
  | { kind: "show"; path: string }
  | { kind: "note"; text: string };

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;

/** `src` as an agent wrote it, for the place at `root` (its absolute path). */
export function chatImageSrc(src: string, alt: string, root: string): ImageSrc {
  const what = alt ? `${alt} (${src})` : src;
  const raw = src.trim();
  if (/^https?:/i.test(raw) || raw.startsWith("//")) return { kind: "note", text: `[remote image not loaded: ${what}]` };
  if (SCHEME.test(raw) && !/^[a-z]:[\\/]/i.test(raw)) return { kind: "note", text: `[image not shown, ${raw.slice(0, raw.indexOf(":"))}: sources are never loaded: ${what}]` };
  let path = raw;
  try { path = decodeURIComponent(raw); } catch { /* an unescaped % is part of the name */ }
  if (path.includes("\0") || path.includes("\\")) return { kind: "note", text: `[image not shown: not a path in this place: ${what}]` };
  const base = root.replace(/\/+$/, "");
  const parts = path.startsWith("/")
    ? (path === base || path.startsWith(base + "/") ? path.slice(base.length + 1).split("/") : null)
    : path.split("/");
  if (!parts || parts.some((p) => p === "..") ) return { kind: "note", text: `[image not shown: outside this place: ${path}]` };
  const ext = /\.([^./]+)$/.exec(path)?.[1]?.toLowerCase();
  if (!ext || !IMAGE_EXTS.includes(ext)) return { kind: "note", text: `[${alt || path}]` };
  return { kind: "show", path };
}

/** A Read call that opened an image: the path in its summary, or null. The
 *  summary of a Read is its `file_path` (`turns::summary_of`). */
export function readImagePath(name: string, summary: string | undefined, failed: boolean | undefined): string | null {
  if (name !== "Read" || !summary || failed) return null;
  const ext = /\.([^./\s]+)$/.exec(summary)?.[1]?.toLowerCase();
  return ext && IMAGE_EXTS.includes(ext) ? summary : null;
}
