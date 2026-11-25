// How a picture that was asked for is DRAWN: the note while it loads or when
// it was refused, the image, the click to enlarge. What asks for it is the
// caller's: ImageView.tsx through the engine door, GuestImage.tsx through a
// guest connection, which may not import the door. Nothing here asks anything.
import { useEffect, useState, type ReactNode } from "react";
import { basename, humanSize } from "./filekind";
import type { FileBlob } from "./imagecache";
import { useEscape } from "./useEscape";

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

export function ImagePicture({ blob, err, path, mime, inline = false, alt = "", enlarge = false, thumb = false, errPrefix = "" }: {
  blob: (FileBlob & { mime?: string }) | null; err: string | null;
  path: string; mime?: string; inline?: boolean; alt?: string; enlarge?: boolean; thumb?: boolean; errPrefix?: string;
}) {
  const [actual, setActual] = useState(false);
  const [big, setBig] = useState(false);
  const [dim, setDim] = useState<{ w: number; h: number } | null>(null);
  useEffect(() => { setDim(null); }, [path]);

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
