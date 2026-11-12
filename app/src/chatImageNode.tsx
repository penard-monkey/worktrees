// `![alt](src)` in a chat bubble (feat/chat-images): the node the chat's
// `Markdown` draws for it. Its own file so `chatimage-check.mjs` renders the
// real thing, through the real `Markdown`.
import type { ReactNode } from "react";
import { ImageView, type PlaceRef } from "./ImageView";
import { chatImageSrc } from "./imgsrc";

/** A picture inside THIS place shows (click to enlarge); every other source is
 *  a note that names it. The host judges the path again (`read_place_image`). */
export function chatImageNode(src: string, alt: string, root: string, place: PlaceRef): ReactNode {
  const r = chatImageSrc(src, alt, root);
  if (r.kind === "note") return <em className="md-img-note" data-testid="chat-img-note">{r.text}</em>;
  return <ImageView key={r.path} path={r.path} place={place} inline enlarge alt={alt} errPrefix="image not shown — " />;
}
