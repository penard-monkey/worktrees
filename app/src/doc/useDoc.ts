// One open document: its session (ideasave.ts: when it saves, and what a change
// made elsewhere means), its editor, its history and its name. Both windows use
// this hook and draw it with DocumentPane.tsx; neither has a second path that
// edits a document. The editor's text reaches the file only through `Session`.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useArmed } from "../useArmed";
import type { EditorHandle, EditorHooks } from "./editortypes";
import { Session, type Changed, type SaveState } from "./ideasave";
import { ideaLink, mergedNote, pickerMatches, versionWhen } from "./ideasrelate";
import type { Doc, DocRows, DocSource, Version } from "./source";

export const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

/** The open document. `focus`: put the cursor in the editor as it opens (a new
 *  one), from the editor host's own effect — a timer set beside the state
 *  change can fire before React commits it (it did, in Chromium). `key` is
 *  this OPENING: the same document read again is a new one. */
export type Open = { id: string; title: string; file: string; key: string; focus: boolean; kind: "idea" | "plan" };
export type Picker = Parameters<NonNullable<EditorHooks["picker"]>>[0];

const clock = {
  setTimeout: (fn: () => void, ms: number) => window.setTimeout(fn, ms),
  clearTimeout: (h: unknown) => window.clearTimeout(h as number),
  now: () => Date.now(),
};

/** The text of the document that is open, for the moments its editor is not:
 *  before the editor has been fetched, between two documents, while the pane is
 *  being torn down. Keyed by the opening, so it is never another document's. */
export type Held = { key: string; text: string };

export type DocOptions = {
  source: DocSource;
  /** Say one line to the person; `null` clears it. */
  notice(msg: string | null): void;
  /** A line for the log: an act and its outcome, never the document's text. */
  trace(msg: string): void;
  /** The list of documents may have moved: pull it again. */
  listed(): void;
  /** Every document there is, for the `[[` picker. */
  rows: DocRows;
  /** A link to another document was followed. */
  follow(id: string): void;
  /** The selection, or the text under one, moved. */
  selectionMoved?(): void;
  /** The text was replaced under the editor (a reload, Claude's edit). */
  replaced?(): void;
  /** A document is now open in place of the one before it. */
  began?(): void;
};

export function useDoc(o: DocOptions) {
  const cb = useRef(o);
  cb.current = o;
  const { source } = o;
  const [open, setOpenState] = useState<Open | null>(null);
  const openRef = useRef<Open | null>(null);
  const setOpen = useCallback((next: Open | null | ((o: Open | null) => Open | null)) => {
    openRef.current = typeof next === "function" ? next(openRef.current) : next;
    setOpenState(openRef.current);
  }, []);
  const [save, setSave] = useState<SaveState>({ kind: "clean", savedAt: null });
  const [mode, setMode] = useState<"edit" | "read">("edit");
  const [readBody, setReadBody] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [hasSel, setHasSel] = useState(false);
  const [versions, setVersions] = useState<Version[] | null | false>(false);
  const [tick, setTick] = useState(0);
  const session = useRef<Session | null>(null);
  const handle = useRef<EditorHandle | null>(null);
  const held = useRef<Held | null>(null);
  const keySeq = useRef(0);
  const readSeq = useRef(0);
  const arm = useArmed<string>();
  const bump = useCallback(() => setTick((n) => n + 1), []);

  // What changed in the source: the open document's session decides what it
  // means for the editor, and whoever shows a list pulls it again.
  useEffect(() => source.watch((ev: Changed) => {
    cb.current.listed();
    bump();
    if (session.current) void session.current.external(ev);
  }), [source, bump]);

  /** Start a session on a document just read, created or changed by an act. */
  const begin = useCallback((d: Doc, focus = false) => {
    readSeq.current++;
    session.current?.close();
    const key = `${d.id}#${++keySeq.current}`;
    const mine: Held = { key, text: d.content };
    held.current = mine;
    let known = { file: d.file, title: d.title };
    const s = new Session(d.id, d.content, d.mtime_ms, {
      rows: {
        write: (id, content, expectedMtime, keepHistory) => source.write(id, content, expectedMtime, keepHistory),
        read: (id) => source.read(id),
      },
      clock,
      refusals: source.refusals,
      // The editor of THIS opening, else the text held for it: the editor of
      // the document before is on screen until React commits this one, and
      // until the editor has been fetched there is none.
      text: () => (handle.current?.key === key ? handle.current.text() : mine.text),
      replace: (c) => { mine.text = c; if (handle.current?.key === key) handle.current.replace(c); setReadBody(c); },
      onState: (st) => {
        setSave(st);
        if (st.kind === "conflict" || st.kind === "removed" || st.kind === "error") cb.current.trace(`${source.noun} ${d.id} ${st.kind}${st.kind === "conflict" ? ` (${st.why})` : ""}`);
      },
      // The first heading names the file: a save may have renamed it. The
      // open document's name and title follow at once, and the list re-pulls
      // rather than wait for the poll.
      onSaved: (saved) => {
        setOpen((cur) => (cur && cur.id === d.id ? { ...cur, file: saved.file, title: saved.title } : cur));
        if (saved.file !== known.file || saved.title !== known.title) {
          if (saved.file !== known.file) cb.current.trace(`saved ${d.id} as ${saved.file}`);
          known = saved;
          cb.current.listed();
        }
      },
      onTaken: ({ file, title }) => {
        if (file === undefined || title === undefined) return;
        known = { file, title };
        setOpen((cur) => (cur && cur.id === d.id ? { ...cur, file, title } : cur));
      },
    });
    session.current = s;
    setSave({ kind: "clean", savedAt: null });
    setRenaming(null);
    arm.disarm();
    setOpen({ id: d.id, title: d.title, file: d.file, key, focus, kind: d.kind === "plan" ? "plan" : "idea" });
    setReadBody(d.content);
    setVersions((v) => (v === false ? false : null));
    setHasSel(false);
    setPicker(null);
    cb.current.began?.();
    bump();
  }, [arm, bump, setOpen, source]);

  /** The open document's id NOW, before React has drawn it. */
  const current = useCallback(() => openRef.current?.id ?? null, []);

  /** Leave the open document, saving first. Refused while its text cannot be
   *  saved (a conflict, or it was removed): that text exists nowhere else. */
  const leave = useCallback(async (): Promise<boolean> => {
    const s = session.current;
    if (!s) return true;
    await s.flush();
    if ((s.held() || s.current.kind === "error") && s.dirty()) {
      cb.current.notice(`This ${source.noun} has text that is not saved. Keep yours or reload before you open another.`);
      return false;
    }
    return true;
  }, [source]);

  /** Open one by its id. `true`: it (or what took it) is the open one now. */
  const openDoc = useCallback(async (id: string): Promise<boolean> => {
    if (openRef.current?.id === id) return true;
    const n = ++readSeq.current;
    if (!(await leave())) return false;
    try {
      const d = await source.read(id);
      // Another one was asked for meanwhile: the last ask wins.
      if (n !== readSeq.current) return false;
      // An id that was merged away lands on the one that took it.
      cb.current.notice(d.merged ? mergedNote(d.merged, d.title) : null);
      if (d.merged) cb.current.trace(`opened ${id}, merged into ${d.id}`);
      if (d.id !== openRef.current?.id) {
        // Typed while the read was out.
        if (!(await leave())) return false;
        if (n !== readSeq.current) return false;
        begin(d);
      }
      return true;
    } catch (e) {
      if (n === readSeq.current) cb.current.notice(`Could not open it: ${errText(e)}`);
      cb.current.listed();
      return false;
    }
  }, [begin, leave, source]);

  const rename = useCallback(async (title: string) => {
    const s = session.current;
    const cur = openRef.current;
    if (!s || !cur || !source.rename) return;
    setRenaming(null);
    if (!title.trim() || title.trim() === cur.title) return;
    await s.flush();
    if (s.held() || s.current.kind === "error") { cb.current.notice(`Save or reload this ${source.noun} before renaming it.`); return; }
    setBusy(true);
    try {
      const d = await source.rename(cur.id, title.trim(), s.lastMtime);
      s.adopt(d.content, d.mtime_ms);
      setOpen((x) => (x && x.id === cur.id ? { ...x, title: d.title, file: d.file } : x));
      cb.current.trace(`renamed ${cur.id} → ${d.file}`);
      cb.current.listed();
    } catch (e) { cb.current.notice(`Could not rename it: ${errText(e)}`); }
    finally { setBusy(false); }
  }, [setOpen, source]);

  /** A removed document's text, kept as a new one. */
  const saveAsNew = useCallback(async () => {
    const cur = openRef.current;
    if (!cur || !source.keepAsNew) return;
    const text = handle.current?.key === cur.key ? handle.current.text() : held.current?.key === cur.key ? held.current.text : "";
    try {
      const d = await source.keepAsNew(cur.title, text);
      cb.current.trace(`kept removed ${cur.id} as ${d.id}`);
      begin(d);
      cb.current.listed();
    } catch (e) { cb.current.notice(`Could not save it as a new ${source.noun}: ${errText(e)}`); }
  }, [begin, source]);

  const closeOpen = useCallback(() => {
    readSeq.current++;
    session.current?.close();
    session.current = null;
    held.current = null;
    setOpen(null);
  }, [setOpen]);

  const restore = useCallback((v: Version) => arm.confirm(`restore:${v.version}`, async () => {
    const cur = openRef.current;
    if (!cur) return;
    const s = session.current;
    if (s) {
      await s.flush();
      if (s.held() || s.current.kind === "error") { cb.current.notice(`Save or reload this ${source.noun} before restoring a version.`); return; }
    }
    try {
      const d = await source.restore(cur.id, v.version, s?.lastMtime);
      cb.current.trace(`restored ${cur.id} ${v.version}`);
      begin(d);
      cb.current.notice(`Restored the version of ${versionWhen(v.at)}. The text it replaced is in Versions.`);
      cb.current.listed();
    } catch (e) { cb.current.notice(`Could not restore it: ${errText(e)}`); }
  }), [arm, begin, source]);

  // The Versions list follows the open document, and any change in the source.
  useEffect(() => {
    if (!open || versions === false) return;
    let gone = false;
    source.versions(open.id).then((v) => { if (!gone) setVersions(v); }).catch((e) => { if (!gone) cb.current.notice(`Could not read its versions: ${errText(e)}`); });
    return () => { gone = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open?.id, versions === false, tick, source]);

  // ── the `[[` picker
  const [picker, setPicker] = useState<Picker>(null);
  const [pickIdx, setPickIdx] = useState(0);
  const pickerDismissed = useRef<number | null>(null);
  const pickRows = useMemo(() => (picker ? pickerMatches(o.rows, picker.query, open?.id ?? null) : []), [picker, o.rows, open]);
  const pickRef = useRef({ picker, pickRows, pickIdx });
  pickRef.current = { picker, pickRows, pickIdx };
  const insertLink = useCallback((r: { id: string | null; title: string }) => {
    const p = pickRef.current.picker;
    if (!p || !r.id) return;
    handle.current?.insert(p.from, p.to, ideaLink(r.title, r.id));
    cb.current.trace(`linked ${r.id} into ${openRef.current?.id}`);
    setPicker(null);
  }, []);

  /** What the editor calls; stable for the pane's life. */
  const hooks = useMemo<EditorHooks>(() => ({
    edited: () => session.current?.edited(),
    blurred: () => { void session.current?.flush(); },
    picker: (p) => {
      if (p && pickerDismissed.current === p.from) return;
      if (!p) pickerDismissed.current = null;
      setPicker((cur) => {
        if (!p) return null;
        if (!cur || cur.query !== p.query) setPickIdx(0);
        return p;
      });
    },
    pickerKey: (k) => {
      const { picker: p, pickRows: rows, pickIdx: i } = pickRef.current;
      if (!p) return false;
      if (k === "Escape") { pickerDismissed.current = p.from; setPicker(null); return true; }
      if (k === "ArrowDown") { setPickIdx((i + 1) % Math.max(1, rows.length)); return true; }
      if (k === "ArrowUp") { setPickIdx((i - 1 + rows.length) % Math.max(1, rows.length)); return true; }
      if ((k === "Enter" || k === "Tab") && rows[i]) { insertLink(rows[i]); return true; }
      return false;
    },
    follow: (id) => cb.current.follow(id),
    selected: (h) => setHasSel(h),
    selectionMoved: () => cb.current.selectionMoved?.(),
  }), [insertLink]);
  const replaced = useCallback(() => cb.current.replaced?.(), []);

  const switchMode = useCallback((m: "edit" | "read") => {
    if (m === "read") setReadBody(handle.current?.text() ?? held.current?.text ?? "");
    setMode(m);
    if (m === "edit") window.setTimeout(() => handle.current?.focus(), 0);
  }, []);

  // A hidden or closing window is a save point, and so is the pane's own end.
  useEffect(() => {
    const flush = () => { void session.current?.flush(); };
    const vis = () => { if (document.visibilityState === "hidden") flush(); };
    document.addEventListener("visibilitychange", vis);
    window.addEventListener("pagehide", flush);
    window.addEventListener("beforeunload", flush);
    return () => {
      flush();
      document.removeEventListener("visibilitychange", vis);
      window.removeEventListener("pagehide", flush);
      window.removeEventListener("beforeunload", flush);
    };
  }, []);

  return {
    source, open, save, mode, setMode, switchMode, readBody, renaming, setRenaming, busy, setBusy, hasSel,
    versions, setVersions, tick, bump, session, handle, held, arm,
    begin, current, leave, openDoc, rename, saveAsNew, closeOpen, restore,
    picker, pickIdx, pickRows, insertLink, hooks, replaced,
  };
}
export type DocHandle = ReturnType<typeof useDoc>;
