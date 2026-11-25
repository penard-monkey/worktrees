// A menu by keyboard: every menu in the app (CtxMenu.tsx, its cascading lists, the
// document's own). Escape is useEscape.ts's; this is the rest of the menu pattern:
//
//  - Opened with a KEY, its first act has the focus. Opened with the pointer, the
//    focus stays where it was (a terminal keeps it, a selection is not dropped) and
//    ↓ goes in.
//  - ↑ ↓ walk the acts and wrap, Home and End are the ends, → opens a cascading row,
//    ← leaves the list it opened. Tab closes: it never goes behind the menu.
//  - However it closes, the focus is back on what had it, unless an act gave it to
//    something else (a field, a dialog).
//
// One CAPTURE listener and a stack, as useEscape has and for its reasons: xterm stops
// every key it handles, and only the top menu takes one.
import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";

type Entry = { el: () => HTMLElement | null; close: () => void; left?: () => void };
const stack: Entry[] = [];

/** The last thing a person did was a key. */
let byKey = false;
if (typeof window !== "undefined") {
  window.addEventListener("keydown", () => { byKey = true; }, true);
  window.addEventListener("pointerdown", () => { byKey = false; }, true);
}

const ITEMS = ".pop-item:not(:disabled), .ix-pop-item:not(:disabled)";
const itemsOf = (el: HTMLElement) => [...el.querySelectorAll<HTMLElement>(ITEMS)].filter((i) => i.offsetParent !== null || i.getClientRects().length > 0);
const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

function onKey(e: KeyboardEvent) {
  const top = stack[stack.length - 1], el = top?.el();
  if (!top || !el || e.isComposing || e.metaKey || e.ctrlKey || e.altKey) return;
  const inside = el.contains(document.activeElement);
  // A field in the menu has its own keys.
  if (inside && typing(document.activeElement)) return;
  const take = () => { e.preventDefault(); e.stopPropagation(); };
  if (e.key === "Tab") { take(); top.close(); return; }
  const items = itemsOf(el), at = items.indexOf(document.activeElement as HTMLElement);
  const go = (i: number) => { take(); items[(i + items.length) % items.length]?.focus({ preventScroll: false }); };
  if (e.key === "ArrowLeft" && top.left) { take(); top.left(); return; }
  if (items.length === 0) return;
  if (e.key === "ArrowDown") go(at < 0 ? 0 : at + 1);
  else if (e.key === "ArrowUp") go(at < 0 ? items.length - 1 : at - 1);
  else if (e.key === "Home" && inside) go(0);
  else if (e.key === "End" && inside) go(items.length - 1);
  else if (e.key === "ArrowRight" && at >= 0 && items[at].getAttribute("aria-haspopup") === "menu") { take(); items[at].click(); }
}

export function useMenuKeys(ref: RefObject<HTMLElement | null>, close: () => void, o: { active?: boolean; left?: () => void } = {}) {
  const { active = true } = o;
  const now = useRef({ close, left: o.left });
  now.current = { close, left: o.left };
  const hasLeft = !!o.left;
  /** What had the focus as it opened (StrictMode opens it twice: never its own act). */
  const opener = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    if (!active) return;
    const el = ref.current;
    const at = document.activeElement as HTMLElement | null;
    if (at && !el?.contains(at)) opener.current = at;
    const before = opener.current;
    // Its acts are menu items, those drawn later too (React sets no role on them).
    const name = () => { if (ref.current?.getAttribute("role") !== "dialog") for (const i of ref.current?.querySelectorAll<HTMLElement>(".pop-item, .ix-pop-item") ?? []) if (!i.hasAttribute("role")) i.setAttribute("role", "menuitem"); };
    name();
    const mo = new MutationObserver(name);
    if (el) mo.observe(el, { childList: true, subtree: true });
    if (byKey && el && !el.contains(document.activeElement)) (itemsOf(el)[0] ?? el).focus({ preventScroll: true });
    return () => {
      mo.disconnect();
      const held = !!el?.contains(document.activeElement);
      // Once whatever the act opened has had the chance to take the focus.
      window.setTimeout(() => {
        const a = document.activeElement;
        if ((held || !a || a === document.body) && (!a || a === document.body || !a.isConnected) && before?.isConnected && before !== document.body) before.focus({ preventScroll: true });
      }, 0);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once for each time it opens
  }, [active]);
  useEffect(() => {
    if (!active) return;
    const entry: Entry = { el: () => ref.current, close: () => now.current.close(), ...(hasLeft ? { left: () => now.current.left?.() } : {}) };
    stack.push(entry);
    if (stack.length === 1) window.addEventListener("keydown", onKey, true);
    return () => {
      const i = stack.indexOf(entry);
      if (i >= 0) stack.splice(i, 1);
      if (stack.length === 0) window.removeEventListener("keydown", onKey, true);
    };
  }, [active, hasLeft, ref]);
}
