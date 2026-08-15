/** Pointer-driven dragging for the nav — the PLUMBING half (rules live in
 *  `dnd.ts`, which is pure and checked by `scripts/dnd-check.mjs`).
 *
 *  Deliberately NOT HTML5 drag-and-drop, for three reasons this repo already
 *  paid for:
 *
 *  1. It only works here at all because `tauri.conf.json` sets
 *     `dragDropEnabled: false` — a WKWebView-specific workaround (ROADMAP's
 *     "smoke the v0.5.0 dock + drag" item) that also costs the app any chance
 *     of accepting a folder dropped from Finder.
 *  2. The same ROADMAP item records that manual-sort drag is unverifiable in
 *     the mock harness: Playwright cannot drive a WKWebView dataTransfer.
 *     Pointer events it CAN dispatch, so the gap and the drop are testable.
 *  3. Opening a gap under the cursor changes what is under the cursor, and with
 *     dragenter/dragleave that is a feedback loop. Here the hit-test is one
 *     `elementFromPoint` per frame and the gap is compensated for arithmetically
 *     (`naturalTop` in dnd.ts), so it cannot chase itself.
 *
 *  A drag only begins after the pointer travels THRESHOLD px — below that the
 *  gesture stays a click, because every row in this nav is primarily a button. */
import { useCallback, useEffect, useRef, useState } from "react";

export type DragItem =
  | { kind: "place"; repo: string; slug: string }
  | { kind: "project"; root: string };

export type DragSession<T> = { item: DragItem; x: number; y: number; target: T | null };

const THRESHOLD = 4;
/** How close to the scroller's edge the pointer must get to auto-scroll, and
 *  how fast it then goes (px per frame — ~8 at 60fps is a readable crawl). */
const EDGE = 36;
const EDGE_SPEED = 8;

/** Swallow the click that belongs to a gesture that just ended. Capture-phase
 *  and window-wide, because the release can land on ANYTHING — a place row
 *  (whose click would ENTER it), a rail lens, the project header's ✕ — and
 *  per-handler guards cannot cover surfaces they were never added to.
 *
 *  `stillDown` is the Escape case: the drag is dead but the button is not, so
 *  the click arrives only with the eventual release — the disarm timer must
 *  not start until a pointerup has been seen, or a slow release outlives it
 *  and the click of a CANCELLED drag still enters a place. Disarm happens on
 *  the first of: the click itself, 150ms after a pointerup (an order of
 *  magnitude past the pointerup→click gap, far under a deliberate re-click),
 *  or a new pointerdown — once a new press starts, the old release's click can
 *  no longer arrive, and the new press's click must pass. */
function swallowNextClick(stillDown: boolean) {
  let timer = 0;
  const disarm = () => {
    clearTimeout(timer);
    window.removeEventListener("click", swallow, true);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointerdown", disarm, true);
  };
  const swallow = (e: MouseEvent) => { e.stopPropagation(); e.preventDefault(); disarm(); };
  const onUp = () => { clearTimeout(timer); timer = window.setTimeout(disarm, 150); };
  window.addEventListener("click", swallow, true);
  window.addEventListener("pointerup", onUp);
  window.addEventListener("pointerdown", disarm, true);
  if (!stillDown) onUp();
}

export function useNavDrag<T>(opts: {
  scrollRef: { current: HTMLElement | null };
  /** Hit-test + landing calculation. Runs every frame the pointer moves. */
  resolve: (item: DragItem, x: number, y: number) => T | null;
  /** Called on drop with the last resolved target (null = dropped on nothing). */
  commit: (item: DragItem, target: T | null) => void;
  /** The drop zone under the pointer, only when it CHANGES — spring-open hangs
   *  off this rather than off every frame. */
  onZone?: (el: HTMLElement | null, item: DragItem | null) => void;
}) {
  const [drag, setDrag] = useState<DragSession<T> | null>(null);
  // Callbacks are re-created every render; the window listeners are not. Keep
  // the live ones in a ref so a drag started three renders ago still commits
  // against current state instead of a closure from mousedown time.
  const cb = useRef(opts);
  cb.current = opts;
  // The whole gesture, outside React: a drag is a stream of frames and setState
  // per frame is for RENDERING it, not for deciding it.
  const g = useRef<{
    item: DragItem; x0: number; y0: number; live: boolean;
    x: number; y: number; target: T | null; zone: HTMLElement | null;
    raf: number; scroll: number;
  } | null>(null);
  const stopScroll = () => {
    if (!g.current?.scroll) return;
    cancelAnimationFrame(g.current.scroll);
    g.current.scroll = 0;
  };

  /** `stillDown` = the gesture ended while the button was still held (Escape),
   *  which changes when the gesture's trailing click can arrive — see
   *  `swallowNextClick`. */
  const end = useCallback((commit: boolean, stillDown = false) => {
    const s = g.current;
    g.current = null;
    if (s?.scroll) cancelAnimationFrame(s.scroll);
    if (s?.raf) cancelAnimationFrame(s.raf);
    document.body.classList.remove("dragging");
    if (s?.live) {
      swallowNextClick(stillDown);
      cb.current.onZone?.(null, null);
      if (commit) cb.current.commit(s.item, s.target);
    }
    setDrag(null);
  }, []);

  useEffect(() => {
    const apply = () => {
      const s = g.current;
      if (!s?.live) return;
      s.raf = 0;
      const el = document.elementFromPoint(s.x, s.y) as HTMLElement | null;
      const zone = el?.closest<HTMLElement>("[data-drop]") ?? null;
      if (zone !== s.zone) { s.zone = zone; cb.current.onZone?.(zone, s.item); }
      s.target = cb.current.resolve(s.item, s.x, s.y);
      setDrag({ item: s.item, x: s.x, y: s.y, target: s.target });
    };
    // Auto-scroll re-resolves every frame WITHOUT a new pointer event: the list
    // is moving under a stationary pointer, so the landing slot changes anyway.
    const autoscroll = () => {
      const s = g.current;
      const sc = cb.current.scrollRef.current;
      if (!s?.live || !sc) return;
      const r = sc.getBoundingClientRect();
      const up = s.y - r.top, down = r.bottom - s.y;
      const dy = up < EDGE ? -EDGE_SPEED : down < EDGE ? EDGE_SPEED : 0;
      if (!dy) { stopScroll(); return; }
      sc.scrollTop += dy;
      apply();
      s.scroll = requestAnimationFrame(autoscroll);
    };
    const move = (e: PointerEvent) => {
      const s = g.current;
      if (!s) return;
      // A release we never saw (it landed outside the window) must not leave a
      // live drag armed for the NEXT release to commit: no button, no gesture.
      if (!(e.buttons & 1)) { end(false); return; }
      s.x = e.clientX;
      s.y = e.clientY;
      if (!s.live) {
        if (Math.hypot(e.clientX - s.x0, e.clientY - s.y0) < THRESHOLD) return;
        s.live = true;
        // Selection and hover suppression live on `body.dragging` (App.css) —
        // cancelling pointermove has no default action per spec, so there is
        // deliberately no preventDefault here.
        document.body.classList.add("dragging");
      }
      if (!s.raf) s.raf = requestAnimationFrame(apply);
      if (!s.scroll) s.scroll = requestAnimationFrame(autoscroll);
    };
    const up = () => end(true);
    const cancel = () => end(false);
    // `stillDown`: on Escape the button is still held, so the gesture's click
    // arrives only with the eventual release — the swallower must wait for it.
    const key = (e: KeyboardEvent) => { if (e.key === "Escape" && g.current?.live) end(false, true); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key);
    };
  }, [end]);

  /** Arm a drag. Left button only, and never from a control inside the row —
   *  the ⋯/✕/+ buttons keep working as buttons. */
  const arm = useCallback((e: React.PointerEvent, item: DragItem) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button,input,textarea,select,a")) return;
    g.current = {
      item, x0: e.clientX, y0: e.clientY, live: false,
      x: e.clientX, y: e.clientY, target: null, zone: null, raf: 0, scroll: 0,
    };
  }, []);

  return { drag, arm };
}
