// One image viewer for the whole app: the Files pane's image tab, its inline
// `![]()`, and the CHAT's (feat/chat-images). It moved out of FilesPane.tsx so
// the chat shows images by the same component rather than a copy. It ASKS, through
// the engine door; ImagePicture.tsx draws and imagecache.ts remembers, and those
// two a guest's view of a shared session uses as well (GuestImage.tsx).
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
import { useEffect, useState } from "react";
import { invoke } from "./ipc";
import { clearImages, loadImage, peekImage, type FileBlob, type PlaceBlob } from "./imagecache";
import { ImagePicture } from "./ImagePicture";

export { CACHE_MAX_BYTES, FRESH_MS, type FileBlob } from "./imagecache";
export type PlaceRef = { repo: string; slug: string };
const keyOf = (p: PlaceRef, path: string) => `${p.repo}\0${p.slug}\0${path}`;

/** What the cache holds for this image right now, or null. */
export const peekPlaceImage = (p: PlaceRef, path: string): PlaceBlob | null => peekImage(keyOf(p, path));

/** The image, from the cache when it is fresh or the host says `unchanged`,
 *  else from the host. One call in flight per image. Exported for the checks. */
export function loadPlaceImage(p: PlaceRef, path: string, now = Date.now()): Promise<PlaceBlob> {
  return loadImage(keyOf(p, path), (etag) => invoke<PlaceBlob>("read_place_image", { repo: p.repo, slug: p.slug, path, etag }), now);
}

/** The picture a Claude Read was shown (`agent_call_image`, the host's console
 *  only): for a Read of a file OUTSIDE the place. The pixels of a call never
 *  change, so one is kept for good (until the LRU takes it) and asked for once. */
export type CallRef = PlaceRef & { callId: string };
const callKey = (c: CallRef) => `call\0${c.repo}\0${c.slug}\0${c.callId}`;
export const peekCallImage = (c: CallRef): PlaceBlob | null => peekImage(callKey(c));
export function loadCallImage(c: CallRef, now = Date.now()): Promise<PlaceBlob> {
  const key = callKey(c);
  return loadImage(key, () =>
    invoke<{ b64: string; size: number; truncated: boolean; mime: string }>("agent_call_image", { repo: c.repo, slug: c.slug, call_id: c.callId })
      .then((b) => ({ ...b, mtime: 0, etag: key })), now, true);
}

/** Forget everything (the checks). */
export const clearPlaceImages = clearImages;

// Loads bytes as base64 and shows them as a data: URI. Used for an image FILE,
// for a relative ![]() inside a rendered markdown doc (`inline`) and, with
// `place`, for the chat's images and a Read's thumbnail (`enlarge`, `thumb`).
export function ImageView({ path, mime, inline = false, alt = "", onError, place, call, fallbackCall, enlarge = false, thumb = false, errPrefix = "" }: {
  path: string; mime?: string; inline?: boolean; alt?: string; onError?: (e: unknown) => void;
  place?: PlaceRef; call?: CallRef; fallbackCall?: CallRef; enlarge?: boolean; thumb?: boolean; errPrefix?: string;
}) {
  const [blob, setBlob] = useState<(FileBlob & { mime?: string }) | null>(() => (call ? peekCallImage(call) : place ? peekPlaceImage(place, path) : null));
  const [err, setErr] = useState<string | null>(null);
  const repo = place?.repo, slug = place?.slug, callId = call?.callId, fbId = fallbackCall?.callId;

  useEffect(() => {
    let alive = true;
    setErr(null);
    if (repo !== undefined && slug !== undefined && callId !== undefined) {
      const c = { repo, slug, callId };
      setBlob(peekCallImage(c));
      loadCallImage(c)
        .then((b) => { if (alive) setBlob(b); })
        .catch((e) => { if (!alive) return; setBlob(null); setErr(String(e)); onError?.(e); });
    } else if (repo !== undefined && slug !== undefined) {
      // Cached: shown already, and only replaced when the file changed on disk.
      setBlob(peekPlaceImage({ repo, slug }, path));
      loadPlaceImage({ repo, slug }, path)
        .then((b) => { if (alive) setBlob(b); })
        .catch((e) => {
          if (!alive) return;
          // The client cannot predict every host refusal (a link out, a case
          // variant, a moved file). A Read the host calls "outside this place"
          // is shown as if the client had said so: the picture Claude was shown.
          if (fbId !== undefined && /^outside this place/.test(String(e))) {
            const c = { repo, slug, callId: fbId };
            setBlob(peekCallImage(c));
            loadCallImage(c)
              .then((b) => { if (alive) setBlob(b); })
              .catch((e2) => { if (!alive) return; setBlob(null); setErr(String(e2)); onError?.(e2); });
            return;
          }
          setBlob(null); setErr(String(e)); onError?.(e);
        });
    } else {
      setBlob(null);
      invoke<FileBlob>("read_file_base64", { path })
        .then((b) => { if (alive) setBlob(b); })
        .catch((e) => { if (!alive) return; setErr(String(e)); onError?.(e); });
    }
    return () => { alive = false; };
  }, [path, repo, slug, callId, fbId, onError]);

  return <ImagePicture blob={blob} err={err} path={path} mime={mime} inline={inline} alt={alt} enlarge={enlarge} thumb={thumb} errPrefix={errPrefix} />;
}
