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
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - r.width - 8)),
      top: Math.max(4, Math.min(y, window.innerHeight - r.height - 8)),
    });
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
