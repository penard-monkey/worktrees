// A cascading submenu inside a CtxMenu: a row ending in ▸ whose list opens
// BESIDE the menu when the pointer rests on it — the macOS menu habit the owner
// asked for (2026-11-14: "should pop out on hover rather than click"). Until
// then a ▸ row REPLACED the whole menu with a second one on click.
//
// Kept out of CtxMenu.tsx on purpose: ctxmenu-check.mjs evaluates that file's
// source under stubbed hooks, and a portal or a new hook there would break it.
//
// The rules, each from a way a hover menu goes wrong:
//  - It opens after OPEN_MS on the row, so a pointer passing over it on the way
//    to another row does not flash it open.
//  - Resting on ANOTHER row of the parent menu closes it after CLOSE_MS, and
//    reaching the list first cancels that: a diagonal move from the row to the
//    list crosses a neighbour for a moment and must not lose the list.
//  - A click (or Enter, which is a click) opens it at once and never toggles it
//    shut: a hand used to the old click opens it by hover first, and a toggle
//    would close it under that click.
//  - It is a PORTAL at the document's root, `position: fixed`. Inside the
//    parent it would be clipped by `.ctxmenu`'s `overflow-y: auto`, and while
//    the parent's `pop` animation runs, its transform makes the parent the
//    containing block of anything fixed inside it.
//  - Escape closes the list alone (it is above the parent on useEscape's
//    stack) and gives the row its focus back; a keyboard open focuses the
//    list's first item.
//  - The parent's `.menu-catch` still takes every click outside both, so the
//    list needs no dismissal path of its own: an item's own handler closes the
//    whole menu, which unmounts this.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import * as Icons from "./icons";
import { useEscape } from "./useEscape";
import { useMenuKeys } from "./useMenuKeys";

export const OPEN_MS = 120;
export const CLOSE_MS = 280;

export function SubMenu({ label, testId, track, disabled, children }: {
  label: ReactNode;
  testId?: string;
  track?: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  const rowRef = useRef<HTMLButtonElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const focusFirst = useRef(false);
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  const later = (next: boolean, ms: number) => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setOpen(next), ms);
  };
  const hold = () => window.clearTimeout(timer.current);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  // Resting on another row of the PARENT menu closes the list. A native
  // listener on the parent's own element: the list is a portal, so its events
  // never reach it, and the row's own children are skipped.
  useEffect(() => {
    const menu = rowRef.current?.closest(".ctxmenu");
    if (!menu) return;
    const over = (e: Event) => {
      const t = e.target as Element | null;
      if (!t || rowRef.current?.contains(t)) return;
      if (t.closest(".pop-item")) later(false, CLOSE_MS);
    };
    menu.addEventListener("pointerover", over);
    return () => menu.removeEventListener("pointerover", over);
  }, []);

  // Beside the PARENT menu's edge (not the row's, which sits inside its
  // padding), its first item level with the row; flipped to the left where the
  // right edge has no room; clamped like CtxMenu, again on every resize.
  useLayoutEffect(() => {
    if (!open) { setPos(null); return; }
    const row = rowRef.current, list = listRef.current;
    if (!row || !list) return;
    const place = () => {
      const r = row.getBoundingClientRect();
      const m = row.closest(".ctxmenu")?.getBoundingClientRect() ?? r;
      const cs = getComputedStyle(list);
      const inset = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.borderTopWidth) || 0);
      const w = list.offsetWidth, h = list.offsetHeight;
      let left = m.right + 2;
      if (left + w > window.innerWidth - 8) left = Math.max(4, m.left - w - 2);
      const top = Math.max(4, Math.min(r.top - inset, window.innerHeight - h - 8));
      setPos({ left, top });
    };
    place();
    const ro = new ResizeObserver(place);
    ro.observe(list);
    window.addEventListener("resize", place);
    return () => { ro.disconnect(); window.removeEventListener("resize", place); };
  }, [open]);

  useLayoutEffect(() => {
    if (!open || !pos || !focusFirst.current) return;
    focusFirst.current = false;
    listRef.current?.querySelector<HTMLButtonElement>(".pop-item:not(:disabled)")?.focus({ preventScroll: true });
  }, [open, pos]);

  const back = () => { hold(); setOpen(false); rowRef.current?.focus({ preventScroll: true }); };
  useEscape(back, open);
  useMenuKeys(listRef, back, { active: open, left: back });

  return (
    <>
      <button
        ref={rowRef}
        type="button"
        className={"pop-item sub-item" + (open ? " open" : "")}
        data-testid={testId}
        data-track={track}
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        onPointerEnter={() => later(true, OPEN_MS)}
        onClick={(e) => { hold(); if (e.detail === 0) focusFirst.current = true; setOpen(true); }}
      >
        <span className="sub-label">{label}</span>
        <Icons.ChevronRight size={12} />
      </button>
      {open && createPortal(
        <div
          ref={listRef}
          className="ctxmenu ctx-flyout"
          role="menu"
          tabIndex={-1}
          data-testid={testId ? `${testId}-list` : undefined}
          style={pos ?? { left: -9999, top: 0 }}
          onPointerEnter={hold}
          onContextMenu={(e) => e.preventDefault()}
        >
          {children}
        </div>,
        document.body,
      )}
    </>
  );
}
