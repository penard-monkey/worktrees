// The right-click menu SHELL, shared by every menu in the app: the nav's place
// and project menus (App.tsx owns their `Ctx` state machine) and the dock's
// Files tab (FilesPane owns its own). Only the frame lives here — position,
// clamping and the three dismissal paths — so a second menu cannot drift out of
// visual step with the first ones. The items are plain `.pop-item` buttons the
// caller passes as children, styled by App.css.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

// Fixed at the cursor, clamped to the viewport, Esc / click-away /
// right-click-away all close. Top-level (NOT nested in a component body) so its
// position state survives the owner's re-renders.
export function CtxMenu({ x, y, onClose, children }: { x: number; y: number; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  // The clamp must re-run when the menu RESIZES, not only when the cursor moves:
  // `x`/`y` are frozen for the menu's whole life, so a menu that grows after it
  // opens (an item arming into two, a lifecycle row appearing) keeps the `top`
  // computed for its old height — and a menu already clamped flush to the bottom
  // pushes its new last row off the screen with no way to reach it. Hence a
  // ResizeObserver rather than a wider dep list: it covers callers that do not
  // exist yet. `offsetWidth/Height` and not `getBoundingClientRect()`, which
  // measures THROUGH the `pop` keyframe's `scale(0.98)` and reports a box ~2%
  // small on the very first frame.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const clamp = () => setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - el.offsetWidth - 8)),
      top: Math.max(4, Math.min(y, window.innerHeight - el.offsetHeight - 8)),
    });
    clamp();
    const ro = new ResizeObserver(clamp);
    ro.observe(el);
    window.addEventListener("resize", clamp);
    return () => { ro.disconnect(); window.removeEventListener("resize", clamp); };
  }, [x, y]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <>
      <div className="menu-catch" onClick={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }} />
      <div ref={ref} className="ctxmenu" style={pos} onContextMenu={(e) => e.preventDefault()}>
        {children}
      </div>
    </>
  );
}
