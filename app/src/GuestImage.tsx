// The pictures in a shared session, as a GUEST sees them: `shared_image` over
// the guest connection, drawn and cached by the owner's chat's own modules
// (ImagePicture.tsx, imagecache.ts). A guest names the share, the turn that
// shows the picture and the path as the agent wrote it, never a repo or a
// slug: the host finds the place (`serve/shareimage.rs`) and judges the path by
// the rule the owner's chat is under, after checking that turn draws it.
//
// What is drawn: the images of the AGENT's text, and a Read of an image: inside
// the place the file (`shared_image`), outside it the picture Claude was SHOWN,
// from the shared transcript by call id (`shared_call_image`), as the owner's
// chat falls back to `agent_call_image`; and in a PERSON's message, the pictures the owner attached or drew,
// which the host shows only when IT stored them during the share (its upload
// ledger: a guest's own words reach a person's message through *Use these*).
// A person's picture the host will not show looks as it did, its alt text, and
// a host too old to have a row is not an error to read: it looks as it did.
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

const callKeyOf = (at: SharedAt, callId: string) => `guestcall\0${at.host}\0${at.share}\0${at.binding ?? "now"}\0${callId}`;

/** The picture a Read of call `callId` was shown, from the shared transcript.
 *  Its pixels never change, so it is kept and never asked about again.
 *  Exported for the check. */
export function loadSharedCallImage(at: SharedAt, callId: string, now = Date.now()): Promise<PlaceBlob> {
  const link = at.link;
  if (!link) return Promise.reject("not connected");
  return loadImage(callKeyOf(at, callId), () =>
    inTurn(link, () => link.call<Omit<PlaceBlob, "etag" | "mtime">>("shared_call_image", {
      share: at.share, ...(at.binding === undefined ? {} : { binding: at.binding }), call_id: callId,
    })).then((b) => ({ ...b, mtime: 0, etag: "call" })), now, true);
}

/** What a refusal shows: nothing but what stood there before (`plain`), or the
 *  host's words. A host from before the row, and a Read outside the place, are
 *  not errors. Pure, for the check. */
export function refusalShows(e: string, thumb: boolean, person = false): "plain" | "words" {
  if (e.startsWith("unknown command")) return "plain";
  if (person && e.startsWith("not shown in this shared session")) return "plain";
  return thumb && e.startsWith("outside this place") ? "plain" : "words";
}

function GuestImage({ at, turn, path, alt, thumb = false, person = false, callId }: { at: SharedAt; turn: number; path: string; alt: string; thumb?: boolean; person?: boolean; callId?: string }) {
  const [blob, setBlob] = useState<PlaceBlob | null>(() => peekSharedImage(at, path) ?? (callId ? peekImage(callKeyOf(at, callId)) : null));
  const [err, setErr] = useState<string | null>(null);
  const { link, host, share, binding, isMain, up } = at;

  useEffect(() => {
    let alive = true;
    const here: SharedAt = { link, host, share, binding, isMain, up };
    const have = peekSharedImage(here, path) ?? (callId ? peekImage(callKeyOf(here, callId)) : null);
    setErr(null);
    setBlob(have);
    // Offline, the last page read stays as it is (§3.4), its pictures with it.
    if (!up && have) return;
    loadSharedImage(here, turn, path)
      // A Read outside the place: the picture Claude was shown, by its call.
      // Without one (an older host, a Read whose result holds none) it looks as
      // it did: the path's own refusal, which draws no thumbnail.
      .catch((e) => (callId && String(e).startsWith("outside this place") ? loadSharedCallImage(here, callId).catch(() => Promise.reject(e)) : Promise.reject(e)))
      .then((b) => { if (alive) setBlob(b); })
      .catch((e) => {
        if (!alive || (have && link?.state().kind !== "up")) return;
        setBlob(null); setErr(String(e));
      });
    return () => { alive = false; };
  }, [link, host, share, binding, isMain, up, turn, path, callId]);

  if (err !== null && refusalShows(err, thumb, person) === "plain") return thumb ? null : <em>{alt}</em>;
  const pic = <ImagePicture blob={blob} err={err} path={path} inline enlarge alt={alt} thumb={thumb} errPrefix={thumb ? "preview not shown — " : "image not shown — "} />;
  if (!thumb) return pic;
  // A Read's thumbnail says nothing until it is known to be one: whether the
  // file is inside the place is the host's answer.
  return blob || err !== null ? <div className="chat-call-thumb">{pic}</div> : null;
}

/** `![alt](src)` in the agent's text: the picture, or a note that names what
 *  is never asked for (a remote URL, another scheme, `..`, no image). */
export function sharedImageNode(src: string, alt: string, at: SharedAt, turn: number, person = false): ReactNode {
  const r = chatImageSrc(src, alt, null, at.isMain);
  if (r.kind === "note") return <em className="md-img-note" data-testid="chat-img-note">{r.text}</em>;
  return <GuestImage key={r.path} at={at} turn={turn} path={r.path} alt={alt} person={person} />;
}

/** A picture attached to a person's message (`[attached: …]`): a thumbnail
 *  once the host shows it, nothing when it will not. */
export function SharedAttachment({ at, turn, path }: { at: SharedAt; turn: number; path: string }) {
  return <GuestImage key={path} at={at} turn={turn} path={path} alt={`Attached: ${path.split("/").pop()}`} thumb person />;
}

/** A Read's thumbnail: `path` is `readImagePath`'s, or null for no picture;
 *  `callId` the call's, for a file outside the place. */
export function SharedThumb({ at, turn, path, callId }: { at: SharedAt; turn: number; path: string | null; callId?: string }) {
  if (path === null || chatImageSrc(path, "", null, at.isMain).kind !== "show") return null;
  return <GuestImage key={path} at={at} turn={turn} path={path} alt={`Read: ${path.split("/").pop()}`} thumb callId={callId} />;
}
