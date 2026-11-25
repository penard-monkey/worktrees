// A workspace's memory file in the Ideas window (workspace-view-design.md §4.6): it
// comes by Float, with the folder's files beside it as its neighbours, and Dock takes
// it back. The same pane as everywhere (doc/MainDocument.tsx); no row of its own.
import { useCallback, useEffect, useMemo, useState, type RefObject } from "react";
import { invoke, listen } from "../src/ipc";
import { MainDocument, type MainDocApi } from "../src/doc/MainDocument";
import { memoryDocs, type MemoryView } from "../src/doc/memory";
import { sameDoc, type DocTarget } from "../src/doc/source";
import { MemoryFailing, MemoryFiles, MemoryHistory, MemoryPreview } from "../src/WorkspaceMemory";
import { fileOf } from "../src/wsmemory";

export type MemoryOpen = { ws: string; name: string; path: string };
const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

/** The folder as it is now, read again when it changes. */
export function useMemoryView(ws: string | null): { view: MemoryView | null; error: string | null; tick: number } {
  const [got, setGot] = useState<{ ws: string; view: MemoryView | null; error: string | null } | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => (ws ? memoryDocs(ws).watch(() => setTick((n) => n + 1)) : undefined), [ws]);
  useEffect(() => {
    if (!ws) return;
    const un = listen<{ id?: string }>("workspaces:history", (e) => { if (e.payload?.id === ws) setTick((n) => n + 1); });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, [ws]);
  useEffect(() => {
    if (!ws) return;
    let gone = false;
    memoryDocs(ws).view().then((view) => { if (!gone) setGot({ ws, view, error: null }); }, (e) => { if (!gone) setGot((g) => ({ ws, view: g?.ws === ws ? g.view : null, error: errText(e) })); });
    return () => { gone = true; };
  }, [ws, tick]);
  return got && got.ws === ws ? { view: got.view, error: got.error, tick } : { view: null, error: null, tick };
}

/** The list's column while a memory file is open: its neighbours. */
export function MemorySide(props: { at: MemoryOpen; view: MemoryView | null; error: string | null; ago(epoch: number): string; open(path: string): void; all(): void }) {
  return (
    <nav className="ix-list" aria-label={`${props.at.name}'s memory`} data-testid="ideas-memory-side">
      <div className="ix-list-head">
        <span className="ix-seg" role="group" aria-label="Which documents">
          <button className="on" aria-pressed="true" title={props.at.name}>{props.at.name || "Memory"}</button>
          <button aria-pressed="false" data-testid="ideas-all" onClick={props.all}>All ideas</button>
        </span>
      </div>
      <div className="ix-mem-files">
        {props.error ? <p className="ix-quiet ix-list-empty" role="alert">It could not be read: {props.error}</p>
          : props.view ? <MemoryFiles ws={props.at.ws} view={props.view} on={props.at.path} ago={props.ago} open={props.open} made={null} />
          : <p className="ix-quiet ix-list-empty">Reading…</p>}
      </div>
    </nav>
  );
}

export function MemoryDoc(props: {
  at: MemoryOpen;
  view: MemoryView | null;
  tick: number;
  api: RefObject<MainDocApi | null>;
  /** The open one is this path now. */
  go(path: string): void;
  /** Open another file of the folder. */
  follow(path: string): void;
  stuck(title: string | null): void;
  /** A link to an idea, from this file. */
  idea(id: string): void;
  landed(path: string, ok: boolean): void;
  dock(path: string): Promise<string | null>;
  docked(): void;
  closed(): void;
  reveal(path: string): void;
}) {
  const { at } = props;
  const source = memoryDocs(at.ws);
  const target = useMemo<DocTarget>(() => ({ source, name: at.path }), [source, at.path]);
  const follow = useCallback((to: DocTarget) => { if (to.source === source) props.follow(to.name); else props.idea(to.name); }, [source, props]);
  /** The workspace's chats, for what History says was live. */
  const [titles, setTitles] = useState<Map<string, string> | null>(null);
  useEffect(() => {
    let gone = false;
    invoke<{ chats: { id: string; title: string }[] }>("workspace_chats", { id: at.ws }).then((r) => { if (!gone) setTitles(new Map(r.chats.map((c) => [c.id, c.title]))); }, () => {});
    return () => { gone = true; };
  }, [at.ws, props.tick]);
  const file = fileOf(props.view, at.path);
  const picture = !!file && file.kind !== "text";
  const { landed } = props;
  useEffect(() => { if (picture) landed(at.path, true); }, [picture, at.path, landed]);
  if (picture) return <div className="ix-mem-picture"><MemoryPreview ws={at.ws} file={file!} tick={props.tick} reveal={props.view ? () => props.reveal(`${props.view!.dir}/${file!.path}`) : null} /></div>;
  return (
    <MainDocument target={target} shown rows={[]} api={props.api}
      opened={(to) => props.go(to.name)}
      refused={(to) => { if (to.source === source) props.go(to.name); }}
      follow={follow}
      stuck={props.stuck}
      waits={() => {}}
      closed={props.closed}
      title={at.path.startsWith("notes/") ? undefined : at.path.slice(at.path.lastIndexOf("/") + 1)}
      landed={(to, ok) => props.landed(to.name, ok && sameDoc(to, target))}
      strips={<MemoryFailing error={props.view?.commit_error ?? null} />}
      float={(to) => props.dock(to.name)}
      floated={props.docked}
      floatAs={{ label: "Dock", title: `Dock: put it back in ${at.name || "its workspace"}`, asking: "Opening it in the main window…", to: "the main window" }}
      history={(close) => <MemoryHistory ready={async () => !props.api.current || props.api.current.leave()} ws={at.ws} view={props.view} chat={(id) => (titles ? titles.get(id) ?? null : "…")} path={at.path} at={null} tick={props.tick} open={(p) => { close(); props.follow(p); }} close={close} />}
    />
  );
}
