/** A workspace's memory on screen (docs/proposals/workspace-view-design.md §3.3,
 *  §4.4, §4.5): the file list, the folder's History, what is not a text, and the
 *  Overview's column. The texts themselves are the document pane's, on
 *  `memoryDocs` (doc/memory.ts). Module scope: it holds state. */
import { useCallback, useEffect, useMemo, useState } from "react";
import { memoryDocs, type CommitError, type MemoryChange, type MemoryFile, type MemoryFileDiff, type MemoryImage, type MemoryView } from "./doc/memory";
import { isImagePath, linkTo, topicPath } from "./doc/memorypath";
import { readableLinks } from "./doc/ideasrelate";
import { parseUnifiedDiff } from "./diff";
import * as Icons from "./icons";
import { Markdown } from "./markdown";
import { useArmed } from "./useArmed";
import { useEscape } from "./useEscape";
import {
  changeFiles, changedBy, changedUnderWords, changeWords, conflictWords, failingWords, fileGroups, fileName, filesWords, lineDiff, linkText, mayEdit, nameAt, newFilePath, putName,
  revertOf, sizeWords, STATUS_WORDS, titleOf, tooBig, whileLive,
} from "./wsmemory";
import { TIDY_WHAT, whenWords } from "./wschats";
import "./WorkspaceMemory.css";

/** Of what the list does not show, how many it names. */
const SKIPPED_SHOWN = 6;
const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

/** The files: the index, then notes, topics, files; folded where there are many. */
export function MemoryFiles(props: { ws: string; view: MemoryView; on: string | null; ago(epoch: number): string; open(path: string): void; 
  /** A new file was made; null: the list is to read. */
  made: ((path: string) => void) | null }) {
  const { view } = props;
  const [q, setQ] = useState("");
  const [folds, setFolds] = useState<Record<string, boolean>>({});
  const groups = useMemo(() => fileGroups(view.folders, q), [view.folders, q]);
  return (
    <div className="wsm-files" data-testid="memory-files">
      <input className="wsc-filter" data-testid="memory-filter" placeholder={`Filter ${filesWords(view.count)}`} value={q} spellCheck={false} onChange={(e) => setQ(e.currentTarget.value)} />
      {groups.length === 0 && <p className="ws-ov-none">No file matches.</p>}
      {groups.map((g) => {
        const open = folds[g.key + (q ? "?" : "")] ?? g.open;
        return (
          <section key={g.key} className="wsm-group">
            {g.name && (
              <button className="wsm-group-h" aria-expanded={open} data-testid="memory-group" data-group={g.key} onClick={() => setFolds((f) => ({ ...f, [g.key + (q ? "?" : "")]: !open }))}>
                {open ? <Icons.ChevronDown size={10} /> : <Icons.ChevronRight size={10} />} {g.name} <span className="wsm-n">{g.files.length}</span>
              </button>
            )}
            {open && (
              <ul className="wsm-list">
                {g.files.map((f) => (
                  <li key={f.path}>
                    <button className={"wsm-file" + (f.path === props.on ? " on" : "")} data-testid="memory-file" data-path={f.path} title={f.path} aria-current={f.path === props.on || undefined} onClick={() => props.open(f.path)}>
                      <span className="wsm-file-in">
                        <span className="wsm-file-i" aria-hidden="true">{f.kind === "image" ? <Icons.Image size={12} /> : <Icons.FileText size={12} />}</span>
                        <span className="wsm-file-n">{fileName(f)}</span>
                        <span className="wsm-file-s">{f.kind === "text" && f.path !== "MEMORY.md" ? props.ago(Math.floor(f.mtime_ms / 1000)) : sizeWords(f.size)}</span>
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        );
      })}
      {props.made && <NewFile ws={props.ws} made={props.made} />}
      {(view.truncated || view.skipped.length > 0) && (
        <div className="wsm-skipped" data-testid="memory-skipped">
          {view.truncated && <p>Not every file is listed.</p>}
          {view.skipped.length > 0 && (
            <>
              <p>{filesWords(view.skipped.length)} not shown:</p>
              <ul>
                {view.skipped.slice(0, SKIPPED_SHOWN).map((s) => <li key={s.path}><b>{s.path}</b>: {s.why}</li>)}
                {view.skipped.length > SKIPPED_SHOWN && <li title={view.skipped.slice(SKIPPED_SHOWN).map((s) => `${s.path}: ${s.why}`).join("\n")}>and {view.skipped.length - SKIPPED_SHOWN} more</li>}
              </ul>
            </>
          )}
        </div>
      )}
    </div>
  );
}

/** "+ New file": a name, and a text under topics (or where the name says). */
function NewFile({ ws, made }: { ws: string; made(path: string): void }) {
  const [name, setName] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEscape(() => { setName(null); setErr(null); }, name !== null);
  const path = name === null ? null : newFilePath(name, (t) => topicPath(t));
  const make = async () => {
    if (!path || busy) return;
    setBusy(true);
    setErr(null);
    try {
      await memoryDocs(ws).create(path, `# ${titleOf(path)}\n\n`);
      setName(null);
      made(path);
    } catch (e) { setErr(errText(e)); }
    finally { setBusy(false); }
  };
  if (name === null) return <button className="wsm-new" data-testid="memory-new" onClick={() => setName("")}><Icons.Plus size={12} /> New file</button>;
  return (
    <div className="wsm-newf">
      <input className="wsc-filter" data-testid="memory-new-name" autoFocus placeholder="Its name" value={name} spellCheck={false} disabled={busy}
        onChange={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === "Enter") void make(); }} />
      <p className="wsm-skipped" data-testid="memory-new-path">{err ?? (path ? path : name.trim() ? "That is not a file name." : "Under topics, unless the name has a folder.")}</p>
      <div className="ws-ov-line">
        <button className="ctrl sm" data-testid="memory-new-make" disabled={!path || busy} onClick={() => void make()}>Make</button>
        <button className="ctrl sm" onClick={() => { setName(null); setErr(null); }}>Cancel</button>
      </div>
    </div>
  );
}

/** Over a memory file: a long index, who else may write it, and what changed
 *  under the person with nothing of theirs unsaved. */
export function MemoryFailing({ error }: { error: CommitError | null }) {
  return error ? <div className="wsm-long" data-testid="memory-failing" role="alert"><span>{failingWords(error, whenWords(new Date(error.since * 1000).toISOString()))}</span></div> : null;
}
export function MemoryStrips(props: {
  failing: CommitError | null;
  mode: "edit" | "read";
  long: string | null;
  /** The titles of the live chats. */
  live: readonly string[];
  /** Said already, for this file. */
  heard: boolean;
  hear(): void;
  under: { before: string; after: string } | null;
  seen(): void;
  /** Tidy…, beside a long index: `why` it cannot start now, the tidy chat
   *  that runs already (`open`), or null where chats are off. */
  tidy?: { why: string | null; open: string | null; busy: boolean; run(): void; show(conv: string): void } | null;
}) {
  const [diff, setDiff] = useState(false);
  const { under } = props;
  useEffect(() => setDiff(false), [under]);
  const may = props.mode === "edit" && !props.heard ? mayEdit(props.live) : null;
  return (
    <>
      <MemoryFailing error={props.failing} />
      {props.long && (
        <div className="wsm-long" data-testid="memory-long" role="status">
          <span>{props.long}{props.tidy?.why && <> <span className="wsm-change-d" data-testid="memory-tidy-why">{props.tidy.why}</span></>}</span>
          {props.tidy && <TidyButton {...props.tidy} />}
        </div>
      )}
      {may && (
        <div className="wsm-long" data-testid="memory-may-edit" role="status">
          <span>{may} <span className="wsm-change-d">If it does while you have unsaved text, you choose: keep yours, or reload its version.</span></span>
          <button className="ctrl sm" data-testid="memory-may-edit-ok" onClick={props.hear}>Dismiss</button>
        </div>
      )}
      {under && (
        <>
          <div className="wsm-long under" data-testid="memory-under" role="status">
            <span>{changedUnderWords(props.live)}</span>
            <button className="ctrl sm" data-testid="memory-under-diff" aria-expanded={diff} onClick={() => setDiff((d) => !d)}>Diff</button>
            <button className="ctrl sm" data-testid="memory-under-ok" onClick={props.seen}>Dismiss</button>
          </div>
          {diff && <div className="wsm-under"><LineDiff before={under.before} after={under.after} /></div>}
        </>
      )}
    </>
  );
}
/** Tidy…: starts a chat on the engine's tidy file. Its title and its
 *  accessible name are what it does ([`TIDY_WHAT`]); with a tidy chat running
 *  it opens that one instead of refusing. */
function TidyButton(t: { why: string | null; open: string | null; busy: boolean; run(): void; show(conv: string): void }) {
  const running = t.open;
  if (running !== null) return <button className="ctrl sm" data-testid="memory-tidy-open" title="Open the tidy chat that is running" onClick={() => t.show(running)}>Open it</button>;
  return <button className="ctrl sm" data-testid="memory-tidy" data-track="workspace.memory.tidy" title={TIDY_WHAT} aria-label={TIDY_WHAT} disabled={t.busy || t.why !== null} onClick={t.run}>Tidy…</button>;
}
function LineDiff({ before, after }: { before: string; after: string }) {
  const rows = useMemo(() => lineDiff(before, after), [before, after]);
  return (
    <div className="wsm-diff" data-testid="memory-diff" role="table" aria-label="What changed">
      {rows.map((r, i) => r.kind === "gap" ? <div key={i} className="wsm-dl gap" aria-hidden="true">⋯</div> : (
        <div key={i} className={`wsm-dl ${r.kind}`} role="row"><span className="wsm-dn">{r.no}</span><span className="wsm-ds" aria-hidden="true">{r.kind === "add" ? "+" : r.kind === "del" ? "−" : " "}</span><span className="wsm-dt">{r.text || " "}</span></div>
      ))}
    </div>
  );
}

/** Files dropped on or pasted into a memory file: kept under `files/`, each a
 *  link (an image is drawn). One that is refused is said, and the rest go on. */
export async function keepFiles(ws: string, from: string, files: readonly File[]): Promise<{ text: string; refused: string[] }> {
  const put: string[] = [], refused: string[] = [];
  for (const f of files) {
    const name = putName(f.name, f.type, new Date());
    const big = tooBig(name, f.size);
    if (big) { refused.push(big); continue; }
    try {
      const r = await memoryDocs(ws).put(name, new Uint8Array(await f.arrayBuffer()));
      put.push(linkText(from, { path: r.path, name: r.file, size: r.size, mtime_ms: r.mtime_ms, kind: isImagePath(r.path) ? "image" : "other", title: null }, linkTo));
    } catch (e) { refused.push(`${name} was not kept: ${errText(e)}.`); }
  }
  return { text: put.join("\n"), refused };
}

/** A file that is not a text: an image's preview, or its name and size. */
export function MemoryPreview(props: { ws: string; file: MemoryFile; tick: number; reveal: (() => void) | null }) {
  const { file } = props;
  // By what it is of: the one before is never drawn under this one's name.
  const of = `${props.ws}|${file.path}|${file.mtime_ms}`;
  const [got, setGot] = useState<{ of: string; img: MemoryImage } | null>(null);
  const img = got?.of === of ? got.img : null;
  useEffect(() => {
    if (file.kind !== "image") return;
    let gone = false;
    memoryDocs(props.ws).image("", linkTo("", file.path)).then((i) => { if (!gone) setGot({ of, img: i }); });
    return () => { gone = true; };
  }, [props.ws, file.path, file.kind, of, props.tick]);
  return (
    <div className="wsm-preview" data-testid="memory-preview" data-path={file.path}>
      {file.kind === "image" && (img === null ? <p className="ws-ov-none">Reading…</p> : "src" in img ? <img src={img.src} alt={file.name} /> : <p className="ws-ov-none">{img.note}</p>)}
      <p className="wsm-preview-w">{file.path} · {sizeWords(file.size)}</p>
      {props.reveal && <button className="ctrl sm" data-testid="memory-reveal" onClick={props.reveal}>Show in Finder</button>}
    </div>
  );
}

/** One file's diff in one change. */
export function MemoryDiff({ d }: { d: MemoryFileDiff }) {
  const diff = d.diff;
  const p = useMemo(() => (diff.kind === "text" ? parseUnifiedDiff(diff.patch) : null), [diff]);
  if (diff.kind === "binary") return <p className="ws-ov-none wsm-pad">Not a text: {d.added === null ? "its bytes changed" : ""}.</p>;
  if (diff.kind === "image") {
    const side = (name: string, s: typeof diff.before) => (
      <figure className="wsm-side">
        <figcaption>{name}</figcaption>
        {!s ? <p className="ws-ov-none">{name === "Before" ? "It was not there." : "It is gone."}</p> : s.refused ? <p className="ws-ov-none">{s.refused}</p> : <img data-testid="memory-diff-image" src={`data:${s.mime};base64,${s.b64}`} alt={`${d.path}, ${name.toLowerCase()}`} />}
      </figure>
    );
    return <div className="wsm-sides" data-testid="memory-diff">{side("Before", diff.before)}{side("After", diff.after)}</div>;
  }
  return (
    <div className="wsm-diff" data-testid="memory-diff" role="table" aria-label={`Changes to ${d.path}`}>
      {p!.rows.length === 0 && <p className="ws-ov-none wsm-pad">No lines changed.</p>}
      {p!.rows.flatMap((r, i) => {
        const line = (kind: "ctx" | "del" | "add", no: number | null, text: string) => (
          <div key={`${i}${kind}`} className={`wsm-dl ${kind}`} role="row"><span className="wsm-dn">{no ?? ""}</span><span className="wsm-ds" aria-hidden="true">{kind === "add" ? "+" : kind === "del" ? "−" : " "}</span><span className="wsm-dt">{text || " "}</span></div>
        );
        if (r.kind === "gap") return [<div key={i} className="wsm-dl gap" aria-hidden="true">⋯</div>];
        const old = () => line("del", r.oldNo, p!.oldLines[r.oldIdx!] ?? ""), now = (k: "add" | "ctx") => line(k, r.newNo, p!.newLines[r.newIdx!] ?? "");
        return r.kind === "mod" ? [old(), now("add")] : r.kind === "del" ? [old()] : [now(r.kind === "add" ? "add" : "ctx")];
      })}
      {diff.truncated && <p className="ws-ov-none wsm-pad">The rest of it is not shown.</p>}
    </div>
  );
}

const Lines = ({ added, removed }: { added: number | null; removed: number | null }) => (
  <span className="wsm-pm">{added ? <span className="wsm-add">+{added}</span> : null}{removed ? <span className="wsm-del">−{removed}</span> : null}</span>
);

/** Undo a change, or Redo one that was undone: two presses, and a refusal names
 *  the files. Shared by History, the Overview's line and a chat's receipt. */
export function useRevert(ws: string, done: (r: MemoryChange) => void, name: (path: string) => string, ready: () => Promise<boolean>) {
  const arm = useArmed<string>();
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<{ commit: string; text: string } | null>(null);
  const run = (c: { commit: string; revert: string }) => arm.confirm(c.commit, async () => {
    setBusy(true);
    setSaid(null);
    try {
      if (!(await ready())) return;
      const r = await memoryDocs(ws).revert(c.revert);
      if (r.conflicts.length) setSaid({ commit: c.commit, text: conflictWords(r.conflicts.map(name)) });
      else if (r.change) done(r.change);
    } catch (e) { setSaid({ commit: c.commit, text: errText(e) }); }
    finally { setBusy(false); }
  });
  return { armed: arm.armed, busy, said, clear: () => setSaid(null), run };
}
export function RevertButton({ c, rv, what }: { c: MemoryChange; rv: ReturnType<typeof useRevert>; what: string }) {
  const to = revertOf(c);
  if (!to) return null;
  const on = rv.armed === c.commit;
  return (
    <button className={"ctrl sm" + (on ? " wsm-armed" : "")} data-testid="memory-revert" data-redo={to.redo || undefined} disabled={rv.busy} onClick={() => rv.run({ commit: c.commit, revert: to.commit })}
      title={to.redo ? "Put this change back, as a new change" : "Undo it with a new change. Nothing is ever dropped."}>
      {to.redo ? (on ? `Redo ${what}?` : "Redo") : on ? `Undo ${what}?` : "Undo"}
    </button>
  );
}

/** History: the folder's changes (the open file's, or all of them), then one
 *  change, one file of it at a time. */
export function MemoryHistory(props: {
  ws: string;
  /** The folder as it is now: how its notes are called. */
  view: MemoryView | null | undefined;
  /** The open file; `null` asks for all. */
  path: string | null;
  /** A change to show first, by its commit. */
  at: string | null;
  /** Moves with every change of the folder. */
  tick: number;
  open(path: string): void;
  close(): void;
  /** A chat's title, by id; null: it was deleted. */
  chat(id: string): string | null;
  /** The open document is saved, before an Undo. */
  ready(): Promise<boolean>;
}) {
  const { ws, path } = props;
  const src = memoryDocs(ws);
  const [all, setAll] = useState(path === null || props.at !== null);
  const [list, setList] = useState<MemoryChange[] | null>(null);
  const [more, setMore] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [cur, setCur] = useState<MemoryChange | null>(null);
  const [file, setFile] = useState<string | null>(null);
  const [diff, setDiff] = useState<MemoryFileDiff | null>(null);
  const named = (p: string) => nameAt(props.view, p);
  const rv = useRevert(ws, (c) => { setCur(c); setFile(null); }, named, props.ready);
  const want = all ? undefined : path ?? undefined;
  const pick = useCallback((c: MemoryChange) => {
    setCur(c);
    setFile(c.files.some((f) => f.path === path) ? path : changeFiles(c)[0]?.path ?? null);
  }, [path]);

  useEffect(() => {
    let gone = false;
    setErr(null);
    src.log({ path: want }).then((l) => { if (!gone) { setList(l.changes); setMore(l.more); } }, (e) => { if (!gone) { setList([]); setErr(errText(e)); } });
    return () => { gone = true; };
  }, [src, want, props.tick]);
  useEffect(() => {
    if (!props.at) return;
    let gone = false;
    src.change(props.at).then((c) => { if (!gone) pick(c); }, (e) => { if (!gone) setErr(errText(e)); });
    return () => { gone = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the change asked for is the trigger
  }, [src, props.at]);
  // The change on screen follows what became of it (undone, put back).
  useEffect(() => {
    if (!cur) return;
    let gone = false;
    src.change(cur.commit).then((c) => { if (!gone) setCur(c); }, () => {});
    return () => { gone = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [src, props.tick, cur?.commit]);
  const at = cur?.commit;
  useEffect(() => {
    setDiff(null);
    if (!at || !file) return;
    let gone = false;
    src.diff(at, file).then((d) => { if (!gone) setDiff(d); }, (e) => { if (!gone) setErr(errText(e)); });
    return () => { gone = true; };
  }, [src, at, file]);
  const older = () => {
    const last = list?.[list.length - 1];
    if (last) void src.log({ path: want, before: last.commit }).then((l) => { setList((x) => [...(x ?? []), ...l.changes]); setMore(l.more); }, (e) => setErr(errText(e)));
  };

  const head = (
    <div className="wsm-hist-h">
      {cur ? <button className="wsf-link" data-testid="memory-hist-back" onClick={() => setCur(null)}>‹ {all ? "All changes" : "History"}</button> : <span className="wsm-hist-t" title={all ? undefined : path ?? undefined}>{all ? "All changes" : `Changes to ${named(path ?? "")}`}</span>}
      {!cur && path !== null && (
        <span className="agent-view" role="group" aria-label="Which changes">
          <button className={"agent-view-b" + (all ? "" : " on")} aria-pressed={!all} data-testid="memory-hist-file" onClick={() => setAll(false)}>This file</button>
          <button className={"agent-view-b" + (all ? " on" : "")} aria-pressed={all} data-testid="memory-hist-all" onClick={() => setAll(true)}>All changes</button>
        </span>
      )}
      <button className="ctrl sm wsm-hist-x" data-testid="memory-hist-close" onClick={props.close}>Close</button>
    </div>
  );
  return (
    <section className="wsm-hist" data-testid="memory-history" aria-label="History">
      {head}
      {err && <p className="wsm-err" role="alert">{err}</p>}
      {cur ? (
        <div className="wsm-change" data-testid="memory-change" data-commit={cur.commit}>
          <div className={`wsm-change-h ${changeWords(cur).by}`}>
            <span className="wsm-change-w"><b>{changedBy(cur, props.chat)}</b><span className="wsm-change-d">{whenWords(new Date(cur.at * 1000).toISOString())} · {filesWords(cur.files.length)} <Lines added={cur.added} removed={cur.removed} />{cur.reverted_by ? " · undone since" : ""}</span></span>
            <RevertButton c={cur} rv={rv} what="this change" />
          </div>
          {rv.said && <p className="wsm-err" role="alert" data-testid="memory-conflict">{rv.said.text}</p>}
          <ul className="wsm-cfiles">
            {changeFiles(cur).map((f) => (
              <li key={f.path}>
                <button className={"wsm-cfile" + (f.path === file ? " on" : "")} data-testid="memory-change-file" data-path={f.path} title={f.path} aria-current={f.path === file || undefined} onClick={() => setFile(f.path)}>
                  <span className="wsm-file-in"><span className="wsm-cfile-n">{named(f.path)}</span><span className="wsm-cfile-s">{STATUS_WORDS[f.status] ?? ""}</span><Lines added={f.added} removed={f.removed} /></span>
                </button>
              </li>
            ))}
          </ul>
          {file && (
            <div className="wsm-one">
              <div className="wsm-one-h"><span title={file}>{named(file)}</span><button className="wsf-link" data-testid="memory-change-open" onClick={() => props.open(file)}>Open</button></div>
              {diff ? <MemoryDiff d={diff} /> : <p className="ws-ov-none wsm-pad">Reading…</p>}
            </div>
          )}
        </div>
      ) : list === null ? <p className="ws-ov-none wsm-pad">Reading…</p> : list.length === 0 ? <p className="ws-ov-none wsm-pad">No changes yet.</p> : (
        <div className="wsm-log">
          {list.map((c) => (
            <button key={c.commit} className={`wsm-row ${changeWords(c).by}${c.reverted_by ? " undone" : ""}`} data-testid="memory-log-row" data-commit={c.commit} data-kind={c.kind} onClick={() => pick(c)}>
              <span className="wsm-file-in">
                <span className="wsm-row-n">{c.subject}{c.live?.length ? <span className="wsm-row-l"> · {whileLive(c.live, props.chat)}</span> : null}</span>
                {c.reverted_by && <span className="wsm-row-u">undone</span>}
                <Lines added={c.added} removed={c.removed} />
                <span className="wsm-row-d">{whenWords(new Date(c.at * 1000).toISOString())}</span>
              </span>
            </button>
          ))}
          {more && <button className="wsf-more" data-testid="memory-log-more" onClick={older}>Older changes ▸</button>}
        </div>
      )}
    </section>
  );
}

/** The Overview's memory: what last changed it, and the index as it reads, cut
 *  where the column ends. */
export function MemoryCard(props: {
  ws: string;
  view: MemoryView;
  ago(epoch: number): string;
  open(path: string | null): void;
  diff(commit: string): void;
  follow(href: string): void;
  changed(): void;
  chat(id: string): string | null;
  /** The open document is saved, before an Undo. */
  ready(): Promise<boolean>;
}) {
  const { view } = props;
  const last = view.last && view.last.kind !== "made" ? view.last : null;
  const rv = useRevert(props.ws, props.changed, (p) => nameAt(props.view, p), props.ready);
  const [box, setBox] = useState<HTMLElement | null>(null);
  const [cut, setCut] = useState(false);
  useEffect(() => {
    if (!box) return;
    const look = () => setCut(box.scrollHeight > box.clientHeight + 2);
    look();
    const ro = new ResizeObserver(look);
    ro.observe(box);
    if (box.firstElementChild) ro.observe(box.firstElementChild);
    return () => ro.disconnect();
  }, [box]);
  return (
    <>
      <MemoryFailing error={view.commit_error ?? null} />
      {last && (
        <div className="wsm-last" data-testid="memory-last">
          <span className="wsm-last-w">{changedBy(last, props.chat)} {props.ago(last.at)}{props.ago(last.at) === "now" ? "" : " ago"} <span className="wsm-change-d">{filesWords(last.files.length)} <Lines added={last.added} removed={last.removed} /></span></span>
          <button className="ctrl sm" data-testid="memory-last-diff" onClick={() => props.diff(last.commit)}>Diff ▸</button>
          <RevertButton c={last} rv={rv} what="it" />
        </div>
      )}
      {rv.said && <p className="wsm-err" role="alert" data-testid="memory-conflict">{rv.said.text}</p>}
      <div className={"wsm-excerpt" + (cut ? " cut" : "")} ref={setBox} data-testid="memory-excerpt">
        {view.index ? <Markdown src={readableLinks(view.index.content)} onLink={props.follow} /> : <p className="ws-ov-none wsm-pad">The memory has no index: MEMORY.md is gone.</p>}
      </div>
      <button className="wsf-more" data-testid="memory-read-all" onClick={() => props.open(null)}>{cut ? "Read all of it in Memory ▸" : "Open it in Memory ▸"}</button>
    </>
  );
}

/** A memory that could not be read: in its own words, never "no memory". */
export function MemoryUnread(props: { why: string; retry(): void }) {
  return (
    <div className="wsf-empty" data-testid="memory-unread" role="alert">
      <p>The memory could not be read.</p>
      <p className="ws-ov-none">{props.why}</p>
      <button className="ctrl sm" data-testid="memory-retry" onClick={props.retry}>Retry</button>
    </div>
  );
}
