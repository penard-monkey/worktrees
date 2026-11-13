// Ideas relate (§3, §6.1, phase 3): the window's small surfaces for the four
// acts — the drop chooser, the "pick another idea" dialog of the menu path,
// Combine's and Split's asks, the `[[` picker, the Versions list and the
// related ideas of the open idea. The rules are ideasrelate.ts's; every act
// ends in one row call the window runs (`Companion.tsx`'s `act`).
import { useEffect, useMemo, useRef, useState } from "react";
import { useEscape } from "../src/useEscape";
import { CHOICES, HOW, pickerMatches, relatedShown, versionWhen, type Choice, type Row } from "./ideasrelate";

export type Relation = { id: string; title: string; how: string; via?: string };
export type Version = { version: string; at: string; size: number; title: string | null };

/** A popover kept inside the window. */
function place(x: number, y: number, w = 260, h = 160): React.CSSProperties {
  const left = Math.max(8, Math.min(x, window.innerWidth - w - 8));
  const top = Math.max(8, Math.min(y, window.innerHeight - h - 8));
  return { left, top };
}

/** Dropped one idea onto another: Relate (the default, Enter) or Merge into
 *  this. Escape or a click outside chooses nothing. */
export function DropChooser(props: { x: number; y: number; from: string; onto: string; choose(c: Choice | null): void }) {
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => { first.current?.focus({ preventScroll: true }); }, []);
  useEscape(() => props.choose(null), true);
  const words: Record<Choice, string> = { relate: "Relate", merge: "Merge into this" };
  return (
    <>
      <div className="ix-catch" onPointerDown={() => props.choose(null)} />
      <div className="ix-pop" role="menu" data-testid="ideas-chooser" style={place(props.x, props.y)}>
        <p className="ix-pop-head">“{props.from}” onto “{props.onto}”</p>
        {CHOICES.map((c, i) => (
          <button key={c} ref={i === 0 ? first : undefined} role="menuitem" className="ix-pop-item" data-testid={`ideas-choose-${c}`} onClick={() => props.choose(c)}>
            {words[c]}{i === 0 && <span className="ix-pop-key">Enter</span>}
          </button>
        ))}
      </div>
    </>
  );
}

/** The menu path's "pick the other idea": a filter and the list. `confirm`
 *  words make it ask before it acts (Merge). */
export function PickIdea<T extends Row>(props: { title: string; rows: readonly T[]; exclude: string | null; confirm?: (r: T) => string; act(r: T): void; close(): void }) {
  const [q, setQ] = useState("");
  const [picked, setPicked] = useState<T | null>(null);
  useEscape(props.close, true);
  const shown = useMemo(() => pickerMatches(props.rows, q, props.exclude).slice(0, 12), [props.rows, q, props.exclude]);
  const go = (r: T) => (props.confirm ? setPicked(r) : props.act(r));
  return (
    <div className="ix-scrim" onPointerDown={(e) => { if (e.target === e.currentTarget) props.close(); }}>
      <div className="ix-dialog" role="dialog" aria-label={props.title} data-testid="ideas-pick">
        <p className="ix-dialog-head">{props.title}</p>
        {picked && props.confirm ? (
          <>
            <p className="ix-quiet">{props.confirm(picked)}</p>
            <div className="ix-dialog-foot">
              <button className="ix-btn" onClick={() => setPicked(null)}>Back</button>
              <button className="ix-btn ix-btn-primary" autoFocus data-testid="ideas-pick-confirm" onClick={() => props.act(picked)}>Merge</button>
            </div>
          </>
        ) : (
          <>
            <input className="ix-filter" autoFocus placeholder="Filter by title" value={q} data-testid="ideas-pick-filter"
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && shown[0]) go(shown[0]); }} />
            <ul className="ix-pick-rows">
              {shown.map((r) => (
                <li key={r.id!}><button className="ix-pop-item" data-testid="ideas-pick-row" data-idea={r.id!} onClick={() => go(r)}>{r.title || "Untitled"}</button></li>
              ))}
              {shown.length === 0 && <li className="ix-quiet">No other idea matches.</li>}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}

/** Combine asks first: it removes the sources' files (each kept in history). */
export function CombineAsk(props: { titles: string[]; act(title: string): void; close(): void }) {
  const [title, setTitle] = useState("Combined idea");
  useEscape(props.close, true);
  return (
    <div className="ix-scrim" onPointerDown={(e) => { if (e.target === e.currentTarget) props.close(); }}>
      <div className="ix-dialog" role="dialog" aria-label="Combine" data-testid="ideas-combine-ask">
        <p className="ix-dialog-head">Combine {props.titles.length} ideas into a new one</p>
        <p className="ix-quiet">{props.titles.map((t) => `“${t}”`).join(", ")} leave the list; their ids keep pointing at the new idea, and each one's text is kept in Versions.</p>
        <input className="ix-filter" autoFocus value={title} data-testid="ideas-combine-title" onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") props.act(title); }} />
        <div className="ix-dialog-foot">
          <button className="ix-btn" onClick={props.close}>Cancel</button>
          <button className="ix-btn ix-btn-primary" data-testid="ideas-combine-go" onClick={() => props.act(title)}>Combine</button>
        </div>
      </div>
    </div>
  );
}

/** Split, from the menu: the selection becomes a new idea; Extract keeps it
 *  here too. */
export function SplitAsk(props: { preview: string; act(o: { title: string; extract: boolean }): void; close(): void }) {
  const [title, setTitle] = useState("");
  const [extract, setExtract] = useState(false);
  useEscape(props.close, true);
  const go = () => props.act({ title, extract });
  return (
    <div className="ix-scrim" onPointerDown={(e) => { if (e.target === e.currentTarget) props.close(); }}>
      <div className="ix-dialog" role="dialog" aria-label="Split into a new idea" data-testid="ideas-split-ask">
        <p className="ix-dialog-head">Split the selection into a new idea</p>
        <pre className="ix-split-preview">{props.preview.length > 400 ? `${props.preview.slice(0, 400)}…` : props.preview}</pre>
        <input className="ix-filter" autoFocus placeholder="Title (default: its first line)" value={title} data-testid="ideas-split-title"
          onChange={(e) => setTitle(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") go(); }} />
        <label className="ix-check"><input type="checkbox" checked={extract} data-testid="ideas-split-extract" onChange={(e) => setExtract(e.target.checked)} />
          Extract a copy (leave the text here)</label>
        <p className="ix-quiet">{extract ? "This idea is not changed." : "The text here is replaced by a link to the new idea; this version is kept in Versions."}</p>
        <div className="ix-dialog-foot">
          <button className="ix-btn" onClick={props.close}>Cancel</button>
          <button className="ix-btn ix-btn-primary" data-testid="ideas-split-go" onClick={go}>{extract ? "Extract" : "Split"}</button>
        </div>
      </div>
    </div>
  );
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
              <span className="ix-version-size">{v.size < 1024 ? `${v.size} B` : `${(v.size / 1024).toFixed(1)} KB`}</span>
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

/** The open idea's related ideas, written and computed (links included). */
export function RelatedStrip(props: { rels: Relation[]; open(id: string): void }) {
  const shown = relatedShown(props.rels);
  if (shown.length === 0) return null;
  return (
    <div className="ix-related" data-testid="ideas-related">
      <span className="ix-related-head">Related</span>
      {shown.map((r) => (
        <button key={r.id} className="ix-chip" data-testid="ideas-related-chip" data-idea={r.id} data-how={r.how}
          title={`${HOW[r.how] ?? r.how}${r.via ? ` (through an id merged into it)` : ""}`} onClick={() => props.open(r.id)}>
          {r.title || "Untitled"}<span className="ix-chip-how">{HOW[r.how] ?? r.how}</span>
        </button>
      ))}
    </div>
  );
}
