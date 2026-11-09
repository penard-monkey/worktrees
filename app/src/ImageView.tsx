// One image viewer for the whole app: the Files pane's image tab, its inline
// `![]()`, and the CHAT's (feat/chat-images). It moved out of FilesPane.tsx so
// the chat shows images by the same component rather than a copy.
//
// Two sources. A FILE (`read_file_base64`, a path the viewer judged against
// every project the caller may see) is the Files pane's. A PLACE image
// (`read_place_image`) is the chat's: the host resolves the path under that
// place's own root alone, and refuses anything else in words that name the
// path. Only place images are cached, because only the chat re-mounts the same
// picture again and again (a bubble re-rendered by every transcript poll).
// The cache is by path AND etag: a cached image is shown at once, with no
// "loading" line to jump the scroll, and revalidated with one tiny call that
// the host answers `unchanged` when mtime + size still match, so an agent that
// overwrites `render.png` is seen to have, and an unchanged one costs no bytes.
import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "./ipc";
import { basename, humanSize } from "./filekind";
import { useEscape } from "./useEscape";

export type FileBlob = { b64: string; size: number; truncated: boolean; mtime: number };
type PlaceBlob = FileBlob & { mime: string; etag: string; unchanged?: boolean };
export type PlaceRef = { repo: string; slug: string };

/** How much decoded-image text the cache keeps before the oldest go. */
export const CACHE_MAX_BYTES = 24 * 1024 * 1024;
/** A cached image seen this recently is not asked about again. */
export const FRESH_MS = 2000;
type Held = { blob: PlaceBlob; checked: number };
const held = new Map<string, Held>();
const flights = new Map<string, Promise<PlaceBlob>>();
let heldBytes = 0;
const keyOf = (p: PlaceRef, path: string) => `${p.repo}\0${p.slug}\0${path}`;

function remember(key: string, blob: PlaceBlob, now: number) {
  const old = held.get(key);
  if (old) heldBytes -= old.blob.b64.length;
  held.delete(key); // Map order is the LRU order: re-insert at the end
  held.set(key, { blob, checked: now });
  heldBytes += blob.b64.length;
  for (const [k, v] of held) {
    if (heldBytes <= CACHE_MAX_BYTES || k === key) break;
    held.delete(k);
    heldBytes -= v.blob.b64.length;
  }
}

/** What the cache holds for this image right now, or null. Synchronous, so a
 *  re-mounted bubble paints the picture in its first frame. */
export function peekPlaceImage(p: PlaceRef, path: string): PlaceBlob | null {
  return held.get(keyOf(p, path))?.blob ?? null;
}

/** The image, from the cache when it is fresh or the host says `unchanged`,
 *  else from the host. One call in flight per image. Exported for the checks. */
export function loadPlaceImage(p: PlaceRef, path: string, now = Date.now()): Promise<PlaceBlob> {
  const key = keyOf(p, path);
  const have = held.get(key);
  if (have && now - have.checked < FRESH_MS) return Promise.resolve(have.blob);
  const going = flights.get(key);
  if (going) return going;
  const flight = invoke<PlaceBlob>("read_place_image", { repo: p.repo, slug: p.slug, path, etag: have?.blob.etag })
    .then((b) => {
      if (b.unchanged && have) {
        remember(key, have.blob, now);
        return have.blob;
      }
      remember(key, b, now);
      return b;
    })
    .finally(() => { flights.delete(key); });
  flights.set(key, flight);
  return flight;
}

/** Forget everything (the checks). */
export function clearPlaceImages() { held.clear(); flights.clear(); heldBytes = 0; }

/** The picture over the window, closed by a click or Esc. `useEscape` takes the
 *  key, so a terminal under it never sees it. Plain CSS, no library. */
function Lightbox({ src, alt, onClose }: { src: string; alt: string; onClose: () => void }) {
  useEscape(onClose);
  return (
    <div className="img-lightbox" role="dialog" aria-modal="true" aria-label={alt || "Image"} data-testid="img-lightbox" onClick={onClose}>
      <img src={src} alt={alt} />
      <span className="img-lightbox-hint">Click or press Esc to close</span>
    </div>
  );
}

// Loads bytes as base64 and shows them as a data: URI. Used for an image FILE,
// for a relative ![]() inside a rendered markdown doc (`inline`) and, with
// `place`, for the chat's images and a Read's thumbnail (`enlarge`, `thumb`).
export function ImageView({ path, mime, inline = false, alt = "", onError, place, enlarge = false, thumb = false, errPrefix = "" }: {
  path: string; mime?: string; inline?: boolean; alt?: string; onError?: (e: unknown) => void;
  place?: PlaceRef; enlarge?: boolean; thumb?: boolean; errPrefix?: string;
}) {
  const [blob, setBlob] = useState<(FileBlob & { mime?: string }) | null>(() => (place ? peekPlaceImage(place, path) : null));
  const [err, setErr] = useState<string | null>(null);
  const [actual, setActual] = useState(false);
  const [big, setBig] = useState(false);
  const [dim, setDim] = useState<{ w: number; h: number } | null>(null);
  const repo = place?.repo, slug = place?.slug;

  useEffect(() => {
    let alive = true;
    setErr(null);
    if (repo !== undefined && slug !== undefined) {
      // Cached: shown already, and only replaced when the file changed on disk.
      setBlob(peekPlaceImage({ repo, slug }, path));
      loadPlaceImage({ repo, slug }, path)
        .then((b) => { if (alive) setBlob(b); })
        .catch((e) => { if (!alive) return; setBlob(null); setErr(String(e)); onError?.(e); });
    } else {
      setBlob(null); setDim(null);
      invoke<FileBlob>("read_file_base64", { path })
        .then((b) => { if (alive) setBlob(b); })
        .catch((e) => { if (!alive) return; setErr(String(e)); onError?.(e); });
    }
    return () => { alive = false; };
  }, [path, repo, slug, onError]);

  // Inline (inside markdown prose) these must be SPANs: a <div> inside the <p>
  // React puts prose in is invalid nesting the browser reparents.
  const note = (cls: string, body: ReactNode) =>
    inline ? <span className={`tree-note inline-note ${cls}`}>{body}</span> : <div className={`tree-note ${cls}`}>{body}</div>;
  if (err) return note("err-note", `${errPrefix}${err}`);
  if (!blob) return note("", "loading image…");
  // A partial image decodes to garbage or nothing — say so instead of showing it.
  if (blob.truncated) return note("", `image too large to preview (${humanSize(blob.size)}+)`);

  const src = `data:${blob.mime ?? mime ?? "image/png"};base64,${blob.b64}`;
  const label = alt || basename(path);
  const img = (
    <img
      className={"img-el" + (actual ? " actual" : "")}
      src={src}
      alt={label}
      onLoad={(e) => setDim({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })}
    />
  );
  if (inline && enlarge) {
    return (
      <span className={"md-img chat-img" + (thumb ? " chat-img-thumb" : "")} data-testid={thumb ? "chat-img-thumb" : "chat-img"}>
        <button type="button" className="chat-img-btn" aria-label={`Enlarge image: ${label}`} title="Click to enlarge" onClick={() => setBig(true)}>
          <img className="img-el" src={src} alt={label} />
        </button>
        {big && <Lightbox src={src} alt={label} onClose={() => setBig(false)} />}
      </span>
    );
  }
  if (inline) return <span className="md-img">{img}</span>;

  return (
    <div className="imgview">
      <div className="img-stage">{img}</div>
      <div className="img-meta">
        <span>{dim ? `${dim.w} × ${dim.h}` : "—"}</span>
        <span className="dot-sep">·</span>
        <span>{humanSize(blob.size)}</span>
        <span className="dock-spacer" />
        <button className="ctrl sm" onClick={() => setActual((a) => !a)}>{actual ? "Fit" : "1:1"}</button>
      </div>
    </div>
  );
}
