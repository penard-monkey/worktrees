// The update bubble — "a release is out", said once per version, pointing at
// the gear whose dot keeps saying it after the bubble is gone.
//
// Two marks, one fact, on purpose and with different jobs: the dot is the
// DURABLE mark (it stays until you update, and it is the only thing on the
// gear), the bubble is the ANNOUNCEMENT (it shows once per release and is gone
// for that tag the moment you close it, act on it, or open Settings → Updates
// any other way). It is not an Offer: offers are things to set up, live in the
// release notes' band and the dock rail's sparkles list, and are dismissed by
// fingerprint of the machine's state — a release is news, and time-bound.
//
// Not modal, so it does not take Escape: `useEscape` would steal the key from
// the terminal for as long as it was up, and Esc is the key claude users press
// most. It does not take focus either. It is anchored to the gear by FIXED
// position from the gear's rect — the rail is a flex column whose children are
// clipped by nothing today, but a popover living inside a host's box is how the
// identity popover once scrolled a header off screen (AGENTS.md).
import { useLayoutEffect, useRef, useState } from "react";
import * as Icons from "./icons";

const GAP = 10; // px between the gear's edge and the bubble's tail tip

export function UpdateBubble({ tag, appVersion, cliVersion, appStale, cliStale, anchor, side, onUpdate, onNotes, onDismiss, onLift }: {
  tag: string;
  appVersion: string | undefined;
  cliVersion: string | null | undefined;
  appStale: boolean;
  cliStale: boolean;
  /** The gear button. Measured, never assumed: the usage strip below it comes and goes. */
  anchor: HTMLElement | null;
  /** Which edge the rail is on. Mirrored Places puts the gear on the right. */
  side: "left" | "right";
  onUpdate: () => void;
  onNotes: () => void;
  onDismiss: () => void;
  /** Distance from the viewport bottom to just above this bubble (null when
   *  gone). The error/undo float-stack anchors at the same bottom-left corner;
   *  App lifts it by this so the two are one stack and never paint over each
   *  other (AGENTS.md: two fixed elements at the same coordinates). */
  onLift: (px: number | null) => void;
}) {
  const [pos, setPos] = useState<{ x: number; bottom: number; tailY: number } | null>(null);
  const boxRef = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    if (!anchor) return;
    const place = () => {
      const r = anchor.getBoundingClientRect();
      const h = boxRef.current?.offsetHeight ?? 0;
      const bottom = Math.max(8, window.innerHeight - r.bottom);
      // Tail at the gear's vertical centre, measured from the bubble's bottom.
      const tailY = Math.min(Math.max(window.innerHeight - bottom - (r.top + r.height / 2), 10), Math.max(10, h - 10));
      setPos({ x: side === "left" ? r.right + GAP : window.innerWidth - r.left + GAP, bottom, tailY });
      onLift(side === "left" ? bottom + h + 8 : null);
    };
    place();
    // The rail's height moves when the usage strip appears or goes, and the
    // window can resize: either moves the gear without moving this.
    const ro = new ResizeObserver(place);
    ro.observe(anchor.parentElement ?? anchor);
    if (boxRef.current) ro.observe(boxRef.current);
    window.addEventListener("resize", place);
    return () => { ro.disconnect(); window.removeEventListener("resize", place); onLift(null); };
  }, [anchor, side, onLift]);

  const behind: string[] = [];
  if (appStale && appVersion) behind.push(`app ${appVersion}`);
  if (cliStale && cliVersion) behind.push(`CLI ${cliVersion}`);

  return (
    <div
      ref={boxRef}
      className={"upd-bubble " + side}
      role="status"
      aria-live="polite"
      data-testid="update-bubble"
      style={pos
        ? { [side]: pos.x, bottom: pos.bottom, ["--tail-y" as string]: `${pos.tailY}px` }
        : { visibility: "hidden" }}
    >
      <div className="upd-bubble-head">
        <span className="upd-bubble-dot" aria-hidden />
        <span className="upd-bubble-title">worktrees {tag} is out</span>
        <button className="upd-bubble-x" title="dismiss — the gear keeps its dot until you update" aria-label="dismiss" onClick={onDismiss}>
          <Icons.X size={13} />
        </button>
      </div>
      {behind.length > 0 && <div className="upd-bubble-body">You have {behind.join(" and ")}.</div>}
      <div className="upd-bubble-actions">
        <button className="enter-btn sm" onClick={onUpdate}>Update…</button>
        <button className="upd-bubble-link" onClick={onNotes}>What’s new <Icons.ExternalLink size={11} /></button>
      </div>
    </div>
  );
}
