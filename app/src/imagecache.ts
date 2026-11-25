// The cache behind the chat's pictures, by whatever names one: a place and a
// path (ImageView.tsx, the owner's chat), a call id, or a share and a path
// (GuestImage.tsx, a guest's view of it). It asks nothing itself: the caller
// hands it the fetch, so a guest view can use it without reaching the engine
// door. No runtime imports, on purpose: guestview-check.mjs walks the guest
// graph and refuses ipc.ts in it.

export type FileBlob = { b64: string; size: number; truncated: boolean; mtime: number };
export type PlaceBlob = FileBlob & { mime: string; etag: string; unchanged?: boolean };

/** How much decoded-image text the cache keeps before the oldest go. */
export const CACHE_MAX_BYTES = 24 * 1024 * 1024;
/** A cached image seen this recently is not asked about again. */
export const FRESH_MS = 2000;
type Held = { blob: PlaceBlob; checked: number };
const held = new Map<string, Held>();
const flights = new Map<string, Promise<PlaceBlob>>();
let heldBytes = 0;

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

/** What the cache holds under `key` right now, or null. Synchronous, so a
 *  re-mounted bubble paints the picture in its first frame. */
export const peekImage = (key: string): PlaceBlob | null => held.get(key)?.blob ?? null;

/** The image under `key`: from the cache when it is fresh (or `forever`, for
 *  pixels that never change), else from `fetch`, which is given the etag held
 *  and may answer `unchanged`. One call in flight per key. */
export function loadImage(key: string, fetch: (etag?: string) => Promise<PlaceBlob>, now: number, forever = false): Promise<PlaceBlob> {
  const have = held.get(key);
  if (have && (forever || now - have.checked < FRESH_MS)) return Promise.resolve(have.blob);
  const going = flights.get(key);
  if (going) return going;
  const flight = fetch(have?.blob.etag)
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
export function clearImages() { held.clear(); flights.clear(); heldBytes = 0; }
