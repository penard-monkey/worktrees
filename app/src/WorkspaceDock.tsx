/** A place's workspace in the dock, to READ (docs/proposals/workspace-view-design.md
 *  §6): its Memory, Plan and Ideas, each under its own beta. Editing is the
 *  workspace view's: there is one editor per window, and it is there. No chat:
 *  a workspace's chats are never inside a place's view. */
import { useCallback, useEffect, useRef, useState } from "react";
import { ReadView } from "./doc/DocumentPane";
import { memoryDocs, type MemoryView } from "./doc/memory";
import type { Doc, DocTarget } from "./doc/source";
import * as Icons from "./icons";
import { WS_TAB_LABEL } from "./navHistory";
import { MemoryFiles, MemoryPreview, MemoryUnread } from "./WorkspaceMemory";
import { fileOf } from "./wsmemory";

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
export type DockDoc = "memory" | "plan" | "ideas";

/** One document, read as it is: read again when its source says it changed. */
function Reader(props: { target: DocTarget; follow(to: DocTarget): void; linkNote?: (from: string, href: string) => string | null }) {
  const { source, name } = props.target;
  const [doc, setDoc] = useState<Doc | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const seq = useRef(0);
  const read = useCallback(() => {
    const n = ++seq.current;
    source.read(name).then((d) => { if (n === seq.current) { setDoc(d); setErr(null); } }, (e) => { if (n === seq.current) setErr(errText(e)); });
  }, [source, name]);
  useEffect(() => { setDoc(null); setErr(null); read(); return () => { seq.current++; }; }, [read]);
  useEffect(() => source.watch(() => read()), [source, read]);
  if (err) return <div className="wsd-none" role="alert" data-testid="dock-ws-error"><p>It could not be read: {err}</p><button className="ctrl sm" onClick={read}>Retry</button></div>;
  if (!doc) return <p className="wsd-none">Reading…</p>;
  return <div className="wsd-read" data-testid="dock-ws-read" data-doc={doc.id}><ReadView open={{ source, id: doc.id }} text={doc.content} follow={props.follow} linkNote={props.linkNote} /></div>;
}

export function WorkspaceDock(props: {
  ws: { id: string; name: string };
  tabs: readonly DockDoc[];
  tab: DockDoc;
  setTab(t: DockDoc): void;
  /** The workspace's plan, and its linked ideas with their titles. */
  plan: DocTarget | null;
  ideas: readonly { id: string; title: string }[];
  idea: string | null;
  setIdea(id: string): void;
  ideaTarget(id: string): DocTarget;
  memory: { view: MemoryView | undefined; error: string | null; path: string; open(path: string): void; back: (() => void) | null; tick: number; retry(): void; linkNote(from: string, href: string): string | null };
  ago(epoch: number): string;
  follow(to: DocTarget): void;
  edit(tab: DockDoc, name: string | null): void;
  float: ((to: DocTarget) => void) | null;
  reveal(path: string): void;
}) {
  const { ws, tab, memory } = props;
  const [list, setList] = useState(false);
  useEffect(() => setList(false), [tab, ws.id]);
  const file = tab === "memory" ? fileOf(memory.view, memory.path) : null;
  const picture = !!file && file.kind !== "text";
  const target: DocTarget | null = tab === "plan" ? props.plan
    : tab === "ideas" ? (props.idea ? props.ideaTarget(props.idea) : null)
    : memory.view?.made && !picture ? { source: memoryDocs(ws.id), name: memory.path } : null;
  const name = tab === "memory" ? memory.path.slice(memory.path.lastIndexOf("/") + 1)
    : tab === "plan" ? "Plan" : props.ideas.find((i) => i.id === props.idea)?.title ?? (props.idea ? "Idea" : "");
  return (
    <div className="wsd" data-testid="dock-ws" data-ws={ws.id} data-tab={tab}>
      <div className="wsd-h">
        <span className="wsd-name" title={ws.name}><Icons.Layers size={12} /> {ws.name}</span>
        <span className="agent-view" role="group" aria-label="The workspace's documents">
          {props.tabs.map((t) => <button key={t} className={"agent-view-b" + (t === tab ? " on" : "")} aria-pressed={t === tab} data-testid="dock-ws-tab" data-tab={t} onClick={() => props.setTab(t)}>{WS_TAB_LABEL[t]}{t === "ideas" && props.ideas.length ? ` ${props.ideas.length}` : ""}</button>)}
        </span>
      </div>
      {(target || picture || tab === "ideas") && (
        <div className="wsd-doc">
          {tab === "memory" && memory.back && <button className="icon-btn" data-testid="dock-ws-back" title="Back to the file before" onClick={memory.back}><Icons.ChevronLeft size={13} /></button>}
          {tab === "memory"
            ? <button className="wsd-files" data-testid="dock-ws-files" aria-expanded={list} title={memory.path} onClick={() => setList((l) => !l)}>{name} ▾</button>
            : tab === "ideas" && props.ideas.length > 0
              ? <button className="wsd-files" data-testid="dock-ws-ideas" aria-expanded={list} onClick={() => setList((l) => !l)}>{name || "Pick an idea"} ▾</button>
              : <span className="wsd-files">{name}</span>}
          {target && <button className="ctrl sm" data-testid="dock-ws-edit" data-track="dock.workspace.edit" title={`Edit it in ${ws.name}`} onClick={() => props.edit(tab, target.name)}>Edit in {ws.name} ▸</button>}
          {target && props.float && tab !== "memory" && <button className="ix-btn ix-btn-icon ix-tip" data-testid="dock-ws-float" data-tip="Float: open it in the Ideas window" aria-label="Float: open it in the Ideas window" onClick={() => props.float!(target)}><Icons.ExternalLink size={13} /></button>}
        </div>
      )}
      {list && tab === "memory" && memory.view && <div className="wsd-list"><MemoryFiles ws={ws.id} view={memory.view} on={memory.path} ago={props.ago} open={(p) => { setList(false); memory.open(p); }} made={null} /></div>}
      {list && tab === "ideas" && (
        <ul className="wsd-list wsf-ideas">
          {props.ideas.map((i) => <li key={i.id} className={"wsf-idea" + (i.id === props.idea ? " on" : "")}><button className="wsf-idea-open" data-testid="dock-ws-idea" data-idea={i.id} onClick={() => { setList(false); props.setIdea(i.id); }}>{i.title || "Untitled"}</button></li>)}
        </ul>
      )}
      {tab === "memory" && memory.error ? <MemoryUnread why={memory.error} retry={memory.retry} />
        : tab === "memory" && !memory.view ? <p className="wsd-none">Reading…</p>
        : tab === "memory" && !memory.view!.made ? <p className="wsd-none" data-testid="dock-ws-none">No memory yet. It starts with the workspace's first chat.</p>
        : picture ? <MemoryPreview ws={ws.id} file={file!} tick={memory.tick} reveal={() => props.reveal(`${memory.view!.dir}/${file!.path}`)} />
        : target ? <Reader key={target.name} target={target} follow={props.follow} linkNote={tab === "memory" ? memory.linkNote : undefined} />
        : <p className="wsd-none" data-testid="dock-ws-none">{tab === "plan" ? "No plan." : props.ideas.length ? "Pick an idea to read it here." : "No linked ideas."}</p>}
    </div>
  );
}

/** Several workspaces hold this project: which one's documents. */
export function PickPlaceWs(props: { rows: readonly { id: string; name: string }[]; pick(id: string): void }) {
  return (
    <div className="wsd" data-testid="dock-ws-pick">
      <p className="wsd-none">This project is in {props.rows.length} workspaces. Whose documents?</p>
      <div className="wsd-list">{props.rows.map((w) => <button key={w.id} className="pop-item" data-testid="dock-ws-pick-row" data-ws={w.id} onClick={() => props.pick(w.id)}>{w.name}</button>)}</div>
    </div>
  );
}
