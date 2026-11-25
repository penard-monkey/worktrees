// The document pane's small surfaces: the `[[` picker, the header's menu and an
// idea's or a plan's History.
import { useEffect, useRef } from "react";
import { useEscape } from "../useEscape";
import { versionWhen, type Row } from "./ideasrelate";
import type { Version } from "./source";

/** A popover kept inside the window. */
export function place(x: number, y: number, w = 260, h = 160): React.CSSProperties {
  const left = Math.max(8, Math.min(x, window.innerWidth - w - 8));
  const top = Math.max(8, Math.min(y, window.innerHeight - h - 8));
  return { left, top };
}

/** The `[[` picker, under the cursor. Keys come through the editor
 *  (`EditorHooks.pickerKey`); a click picks too. */
export function LinkPicker<T extends Row>(props: { x: number; y: number; rows: T[]; index: number; pick(r: T): void }) {
  return (
    <div className="ix-pop ix-linkpick" role="listbox" data-testid="ideas-link-picker" style={place(props.x, props.y + 4, 280, 220)}
      onMouseDown={(e) => e.preventDefault()}>
      {props.rows.length === 0 && <p className="ix-quiet ix-pop-head">No idea's title matches.</p>}
      {props.rows.map((r, i) => (
        <button key={r.id!} role="option" aria-selected={i === props.index} className={`ix-pop-item${i === props.index ? " on" : ""}`}
          data-testid="ideas-link-pick" onClick={() => props.pick(r)}>{r.title || "Untitled"}</button>
      ))}
    </div>
  );
}

/** One of the header's acts, in its menu. */
export type DocAct = {
  testId: string;
  label: string;
  title?: string;
  disabled?: boolean;
  /** The first press of two. */
  armed?: boolean;
  /** The menu stays up after it (an act that asks first). */
  stays?: boolean;
  /** A press keeps the editor's focus and selection, which it is about (Split…).
   *  Any other press leaves the editor, which saves. */
  keeps?: boolean;
  /** As one of the header's own buttons: before History. */
  first?: boolean;
  run(): void;
};

/** The header's menu. `x` is the menu's RIGHT edge. */
export function DocMenu(props: { x: number; y: number; byKey: boolean; acts: readonly DocAct[]; close(): void }) {
  const ref = useRef<HTMLDivElement>(null);
  useEscape(props.close);
  // Opened with a key: the first act takes the focus.
  useEffect(() => {
    if (props.byKey) ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [props.byKey]);
  return (
    <>
      <div className="ix-catch" onPointerDown={props.close} onContextMenu={(e) => { e.preventDefault(); props.close(); }} />
      <div ref={ref} className="ix-pop ix-menu" role="menu" data-testid="doc-menu" style={place(props.x - 220, props.y, 220, 40 + props.acts.length * 28)}>
        {props.acts.map((a) => (
          <button key={a.testId} className={`ix-pop-item${a.armed ? " ix-btn-armed" : ""}`} role="menuitem" data-testid={a.testId} title={a.title} disabled={a.disabled}
            onMouseDown={a.keeps ? (e) => e.preventDefault() : undefined}
            onClick={() => { if (!a.stays) props.close(); a.run(); }}>{a.label}</button>
        ))}
      </div>
    </>
  );
}

/** The open document's history, newest first, each with Restore (armed). */
export function VersionsPanel(props: { versions: Version[] | null; armed: string | null; restore(v: Version): void; close(): void }) {
  return (
    <section className="ix-versions" data-testid="ideas-versions" aria-label="History">
      <div className="ix-versions-head">
        <span>History</span>
        <button className="ix-btn ix-btn-quiet ix-btn-small" onClick={props.close}>Close</button>
      </div>
      {props.versions === null ? <p className="ix-quiet">Reading…</p> : props.versions.length === 0 ? (
        <p className="ix-quiet">No earlier versions. A copy is kept before every merge, split, rename, restore or remove.</p>
      ) : (
        <ul className="ix-version-rows">
          {props.versions.map((v) => (
            <li key={v.version} className="ix-version" data-testid="ideas-version" data-version={v.version}>
              <span className="ix-version-when">{versionWhen(v.at)}</span>
              <span className="ix-version-title">{v.title ?? "(no title)"}</span>
              <span className="ix-version-size">{v.size < 1024 ? `${v.size} B` : `${(v.size / 1024).toFixed(1)} KB`}</span>
              <button className={`ix-btn ix-btn-small${props.armed === v.version ? " ix-btn-armed" : ""}`} data-testid="ideas-version-restore"
                title="Puts this version back. The current text is kept in History first." onClick={() => props.restore(v)}>
                {props.armed === v.version ? "Restore? Current kept" : "Restore"}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
