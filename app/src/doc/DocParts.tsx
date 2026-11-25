// The document pane's small surfaces: the `[[` picker and the Versions list.
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

/** The open idea's history, newest first, each with Restore (armed). */
export function VersionsPanel(props: { versions: Version[] | null; armed: string | null; restore(v: Version): void; close(): void }) {
  return (
    <section className="ix-versions" data-testid="ideas-versions" aria-label="Versions">
      <div className="ix-versions-head">
        <span>Versions</span>
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
              <span className="ix-version-size">{v.size === undefined ? "" : v.size < 1024 ? `${v.size} B` : `${(v.size / 1024).toFixed(1)} KB`}</span>
              <button className={`ix-btn ix-btn-small${props.armed === v.version ? " ix-btn-armed" : ""}`} data-testid="ideas-version-restore"
                title="Puts this version back. The current text is kept in Versions first." onClick={() => props.restore(v)}>
                {props.armed === v.version ? "Restore? Current kept" : "Restore"}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
