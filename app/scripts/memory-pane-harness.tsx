// A bench for memory-browser.mjs, never built into the app: the REAL document
// pane (useDoc, DocumentPane, Session, the editor) on a memory source, with the
// folder's list, its history, one change's diff and the images, drawn plainly.
// vite serves this file in dev; nothing under app/src imports it.
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { DocumentPane } from "../src/doc/DocumentPane";
import { memoryDocs, type MemoryChange, type MemoryFileDiff, type MemoryImage, type MemoryView } from "../src/doc/memory";
import { useDoc } from "../src/doc/useDoc";

function Bench({ id }: { id: string }) {
  const src = memoryDocs(id);
  const [view, setView] = useState<MemoryView | null>(null);
  const [log, setLog] = useState<MemoryChange[]>([]);
  const [diff, setDiff] = useState<MemoryFileDiff | null>(null);
  const [pics, setPics] = useState<MemoryImage[]>([]);
  const [note, setNote] = useState<string | null>(null);
  const pull = () => { void src.view().then(setView); void src.log().then((l) => setLog(l.changes)); };
  const doc = useDoc({ source: src, notice: setNote, trace: () => {}, listed: pull, rows: [], follow: () => {} });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { pull(); void doc.openDoc("MEMORY.md"); }, []);
  const at = doc.open?.id;
  useEffect(() => {
    if (!at) return;
    const links = [...doc.held.current!.text.matchAll(/!\[[^\]]*\]\(([^)]+)\)/g)].map((m) => m[1]);
    void Promise.all(links.map((l) => src.image(at, l))).then(setPics);
  }, [at, doc.tick, src, doc.held]);
  return (
    <div style={{ position: "fixed", inset: 0, zIndex: 99999, display: "grid", gridTemplateColumns: "220px 1fr 420px", background: "var(--bg-abyss)", color: "var(--txt)" }}>
      <nav style={{ overflow: "auto", padding: 12 }} data-testid="mem-list">
        {view?.folders.map((f) => (
          <div key={f.name}><b>{f.name || "memory"}</b>
            {f.files.map((x) => <div key={x.path}><button data-path={x.path} disabled={x.kind !== "text"} onClick={() => void doc.openDoc(x.path)}>{x.title ?? x.name}</button></div>)}
          </div>
        ))}
      </nav>
      <section className="ix-doc ix-in-main" style={{ minWidth: 0, display: "flex", flexDirection: "column" }}>
        {note && <p data-testid="mem-note">{note}</p>}
        {doc.open && <DocumentPane doc={doc} follow={() => {}} />}
        <div data-testid="mem-pics" style={{ padding: 8 }}>{pics.map((p, i) => ("src" in p ? <img key={i} src={p.src} alt="" height={90} /> : <i key={i}>{p.note}</i>))}</div>
      </section>
      <aside style={{ overflow: "auto", padding: 12 }} data-testid="mem-log">
        {log.map((c) => (
          <div key={c.commit} data-kind={c.kind} data-commit={c.commit} style={{ marginBottom: 8 }}>
            <div>{c.kind} · {c.subject} <small>+{c.added} −{c.removed}</small></div>
            {c.files.map((f) => <button key={f.path} data-file={f.path} onClick={() => void src.diff(c.commit, f.path).then(setDiff)}>{f.path}</button>)}
            <button data-act="revert" onClick={() => void src.revert(c.reverted_by ?? c.commit).then((r) => { setNote(r.conflicts.length ? `conflict: ${r.conflicts.join(", ")}` : null); pull(); }, (e) => setNote(String(e)))}>{c.reverted_by ? "Redo" : "Undo"}</button>
          </div>
        ))}
        {diff?.diff.kind === "text" && <pre data-testid="mem-diff">{diff.diff.patch}</pre>}
        {diff?.diff.kind === "image" && [diff.diff.before, diff.diff.after].map((s, i) => s && <img key={i} data-testid="mem-side" src={`data:${s.mime};base64,${s.b64}`} alt="" height={80} />)}
      </aside>
    </div>
  );
}

export function mount(id: string) {
  const el = document.createElement("div");
  document.body.append(el);
  createRoot(el).render(<Bench id={id} />);
}
