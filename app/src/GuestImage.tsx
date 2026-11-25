// The pictures in a shared session, as a GUEST sees them: `shared_image` over
// the guest connection, drawn and cached by the owner's chat's own modules
// (ImagePicture.tsx, imagecache.ts). A guest names the share, the turn that
// shows the picture and the path as the agent wrote it, never a repo or a
// slug: the host finds the place (`serve/shareimage.rs`) and judges the path by
// the rule the owner's chat is under, after checking that turn draws it.
//
// What is drawn: the images of the AGENT's text, and a Read of one inside the
// place. A Read of a file outside it is the picture only the owner's console is
// given (`agent_call_image`), so it has no thumbnail here, and neither it nor a
// host too old to have the row is an error to read: both look as they did.
//
// One picture in flight per connection. A reply can be 5.3 MB (a 4 MB file in
// base64) and the host closes a connection whose queue passes 8 MiB, so three
// asked at once could end it, and the reconnect would ask again.
import { useEffect, useState, type ReactNode } from "react";
import type { GuestLink } from "./guestdoor";
import { loadImage, peekImage, type PlaceBlob } from "./imagecache";
import { ImagePicture } from "./ImagePicture";
import { chatImageSrc } from "./imgsrc";

/** Where a shared picture is asked for. Stable while these stand, so a
 *  bubble's `renderImage` (and `Markdown`'s memo) is. */
export type SharedAt = {
  link: GuestLink | undefined;
  host: string;
  share: string;
  binding: number | undefined;
  /** The shared place is the project's `(main)`. */
  isMain: boolean;
  /** The connection is up: a picture that failed while it was not is asked for
   *  again when it is. */
  up: boolean;
};

const keyOf = (at: SharedAt, path: string) => `guest\0${at.host}\0${at.share}\0${at.binding ?? "now"}\0${path}`;
export const peekSharedImage = (at: SharedAt, path: string): PlaceBlob | null => peekImage(keyOf(at, path));

const queues = new WeakMap<GuestLink, Promise<unknown>>();
function inTurn<T>(link: GuestLink, f: () => Promise<T>): Promise<T> {
  const run = (queues.get(link) ?? Promise.resolve()).then(f);
  queues.set(link, run.catch(() => {}));
  return run;
}

/** The picture turn `turn` shows at `path`. Exported for the check. */
export function loadSharedImage(at: SharedAt, turn: number, path: string, now = Date.now()): Promise<PlaceBlob> {
  const link = at.link;
  if (!link) return Promise.reject("not connected");
  return loadImage(keyOf(at, path), (etag) =>
    inTurn(link, () => link.call<PlaceBlob>("shared_image", {
      share: at.share, ...(at.binding === undefined ? {} : { binding: at.binding }), turn, path, ...(etag === undefined ? {} : { etag }),
    })), now);
}

/** What a refusal shows: nothing but what stood there before (`plain`), or the
 *  host's words. A host from before the row, and a Read outside the place, are
 *  not errors. Pure, for the check. */
export function refusalShows(e: string, thumb: boolean): "plain" | "words" {
  if (e.startsWith("unknown command")) return "plain";
  return thumb && e.startsWith("outside this place") ? "plain" : "words";
}

function GuestImage({ at, turn, path, alt, thumb = false }: { at: SharedAt; turn: number; path: string; alt: string; thumb?: boolean }) {
  const [blob, setBlob] = useState<PlaceBlob | null>(() => peekSharedImage(at, path));
  const [err, setErr] = useState<string | null>(null);
  const { link, host, share, binding, isMain, up } = at;

  useEffect(() => {
    let alive = true;
    const here: SharedAt = { link, host, share, binding, isMain, up };
    const have = peekSharedImage(here, path);
    setErr(null);
    setBlob(have);
    // Offline, the last page read stays as it is (§3.4), its pictures with it.
    if (!up && have) return;
    loadSharedImage(here, turn, path)
      .then((b) => { if (alive) setBlob(b); })
      .catch((e) => {
        if (!alive || (have && link?.state().kind !== "up")) return;
        setBlob(null); setErr(String(e));
      });
    return () => { alive = false; };
  }, [link, host, share, binding, isMain, up, turn, path]);

  if (err !== null && refusalShows(err, thumb) === "plain") return thumb ? null : <em>{alt}</em>;
  const pic = <ImagePicture blob={blob} err={err} path={path} inline enlarge alt={alt} thumb={thumb} errPrefix={thumb ? "preview not shown — " : "image not shown — "} />;
  if (!thumb) return pic;
  // A Read's thumbnail says nothing until it is known to be one: whether the
  // file is inside the place is the host's answer.
  return blob || err !== null ? <div className="chat-call-thumb">{pic}</div> : null;
}

/** `![alt](src)` in the agent's text: the picture, or a note that names what
 *  is never asked for (a remote URL, another scheme, `..`, no image). */
export function sharedImageNode(src: string, alt: string, at: SharedAt, turn: number): ReactNode {
  const r = chatImageSrc(src, alt, null, at.isMain);
  if (r.kind === "note") return <em className="md-img-note" data-testid="chat-img-note">{r.text}</em>;
  return <GuestImage key={r.path} at={at} turn={turn} path={r.path} alt={alt} />;
}

/** A Read's thumbnail: `path` is `readImagePath`'s, or null for no picture. */
export function SharedThumb({ at, turn, path }: { at: SharedAt; turn: number; path: string | null }) {
  if (path === null || chatImageSrc(path, "", null, at.isMain).kind !== "show") return null;
  return <GuestImage key={path} at={at} turn={turn} path={path} alt={`Read: ${path.split("/").pop()}`} thumb />;
}
