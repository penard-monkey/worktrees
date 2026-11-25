// A document open in the main window (workspace-chats.md §4.5, the workspace
// view's slices 0 and 1): the shared pane under its own header, in the
// workspace view's Plan and Ideas tabs (WorkspaceFrame.tsx), whose tabs are the
// way back. Float MOVES it to the Ideas window.
//
// **It outlives its place on screen.** The main area shows something else the
// moment a place is picked in the nav, with nothing to ask first. So the pane
// stays mounted and is hidden: `shown` false is a save point, and a text that
// cannot be saved (a conflict, a removal) waits there with its banner until the
// person is back.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "../ipc";
import { DocNotice, DocumentPane } from "./DocumentPane";
import { sameDoc, type DocRows, type DocTarget } from "./source";
import { useDoc, type Open, type PickRow } from "./useDoc";

export type MainDocApi = {
  /** Save and say whether it may be left: false while its text is nowhere else. */
  leave(): Promise<boolean>;
  /** Show the open one in Edit. */
  edit(): void;
  /** A person is editing it: on screen, in Edit. */
  editing(): boolean;
};

export function MainDocument(props: {
  target: DocTarget;
  /** Open in Edit, whatever its kind opens in ("Edit the plan"). */
  startIn?: "edit";
  shown: boolean;
  rows: DocRows;
  api: React.RefObject<MainDocApi | null>;
  /** The open one is another than the one asked for: a merged id landed on it. */
  opened(to: DocTarget): void;
  /** The one asked for did not open, because this one's text is not saved. */
  refused(to: DocTarget): void;
  follow(to: DocTarget): void;
  /** Its text is nowhere else (`null`: it is, or it can still be saved). */
  stuck(title: string | null): void;
  /** Off screen with text that is not saved: it waits here. */
  waits(title: string): void;
  /** It is open no more: closed from its banner. */
  closed(): void;
  /** Ask the other window to open it. `null`: it has it open, and this one may
   *  close; else why not, and it stays here. */
  float?(to: DocTarget, title: string): Promise<string | null>;
  /** It was: closed here, or left here to be read. */
  floated?(to: DocTarget, title: string): void;
  /** What the button says, where it is not Float (the Ideas window's Dock). */
  floatAs?: { label: string; title: string; asking: string; to: string };
  /** The document asked for is open (`true`), or could not be. */
  landed?(to: DocTarget, ok: boolean): void;
  lead?: ReactNode;
  title?: string;
  strips?: ReactNode | ((mode: "edit" | "read") => ReactNode);
  attach?: (files: File[]) => Promise<{ text: string; refused: string[] }>;
  picks?: (query: string, from: string) => PickRow[];
  /** It changed on disk under a person with nothing unsaved. */
  changedUnder?: (name: string, before: string, after: string) => void;
  linkNote?: (from: string, href: string) => string | null;
  /** Moves when History is asked for from elsewhere. */
  historyAsk?: number;
  /** History, when it is not the document's own versions. */
  history?: (close: () => void) => ReactNode;
}) {
  const { target } = props;
  const [notice, setNotice] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [again, setAgain] = useState(0);
  const at = useRef(props);
  at.current = props;
  const openRef = useRef<Open | null>(null);
  const doc = useDoc({
    source: target.source,
    notice: setNotice,
    trace: (m) => { invoke("log_event", { level: "info", msg: `document: ${m}` }).catch(() => {}); },
    listed: () => {},
    rows: props.rows,
    picks: props.picks,
    changedUnder: (before, after) => { const t = openRef.current; if (t) at.current.changedUnder?.(t.id, before, after); },
    follow: (to) => at.current.follow(to),
  });
  const refusedNow = useRef(false);
  const { open, openDoc, session, closeOpen, save, current, switchMode } = doc;
  const leaveDoc = doc.leave;
  const leave = useCallback(async (before: string) => {
    const ok = await leaveDoc(before);
    refusedNow.current = !ok;
    return ok;
  }, [leaveDoc]);
  openRef.current = open;
  const modeRef = useRef(doc.mode);
  modeRef.current = doc.mode;
  const had = useRef(false);

  // Open what is asked for, and tell the window when it is another: a merged id
  // lands on the one that took it. One that cannot be opened leaves nothing on
  // screen in its place, least of all the document of the tab before.
  useEffect(() => {
    let gone = false;
    setFailed(false);
    void openDoc(target.name).then(async (ok) => {
      if (gone) return;
      const cur = current();
      if (ok) {
        if (cur && cur.name !== target.name) at.current.opened(cur);
        if (cur) at.current.landed?.(cur, true);
        if (at.current.startIn === "edit") switchMode("edit", false);
        return;
      }
      if (cur && !(await leave("before you open another"))) { if (!gone) at.current.refused(cur); return; }
      if (gone) return;
      had.current = false;
      closeOpen();
      setFailed(true);
      at.current.landed?.(target, false);
    });
    return () => { gone = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the one asked for is the trigger
  }, [target.source, target.name, again]);

  useEffect(() => {
    const api = props.api;
    api.current = { leave: () => leave("before you leave it"), edit: () => switchMode("edit", false), editing: () => at.current.shown && !!openRef.current && modeRef.current === "edit" };
    return () => { api.current = null; };
  }, [props.api, leave, switchMode]);

  // Off screen: a save point. A text with nowhere to go waits here.
  useEffect(() => {
    if (props.shown) return;
    let back = false;
    void leave("before you leave it").then((ok) => {
      if (!ok && !back) at.current.waits(openRef.current?.title || "Untitled");
    });
    return () => { back = true; };
  }, [props.shown, leave]);

  const { setHistory } = doc;
  useEffect(() => { if (props.historyAsk) setHistory(true); }, [props.historyAsk, setHistory]);

  const stuck = open && (save.kind === "conflict" || save.kind === "removed" || save.kind === "error") && session.current?.dirty() ? open.title || "Untitled" : null;
  // "Not saved, keep yours or reload" is answered by its being saved.
  useEffect(() => { at.current.stuck(stuck); if (!stuck && refusedNow.current) { refusedNow.current = false; setNotice(null); } }, [stuck]);
  useEffect(() => () => { at.current.stuck(null); }, []);

  // The last thing open was closed from its banner (it had been removed).
  useEffect(() => {
    if (open) had.current = true;
    else if (had.current) { had.current = false; at.current.closed(); }
  }, [open]);

  /** Float is a MOVE: saved, then opened there, then closed here. It stays on
   *  screen until the other window says it has it; a refusal there, a window that
   *  did not open and one that did not answer are said here, and nothing moves. */
  const [asking, setAsking] = useState(false);
  const asked = useRef(false);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const float = async () => {
    const to = current();
    if (!to || !at.current.float || asked.current || !(await leave("before you float it"))) return;
    if (!sameDoc(current(), to)) return;
    const title = openRef.current?.title || "Untitled";
    const key = openRef.current?.key;
    asked.current = true;
    setAsking(true);
    const why = await at.current.float(to, title);
    asked.current = false;
    if (!alive.current) return;
    setAsking(false);
    if (why !== null) { setNotice(`${why} “${title}” is still here.`); return; }
    // Typed while it was asked: saved first, and the other window reads it again.
    const mine = openRef.current?.key === key;
    if (mine && !(await leave("before it closes here: it is open in the other window too"))) return;
    if (mine && openRef.current?.key === key) { had.current = false; closeOpen(); }
    at.current.floated?.(to, title);
  };
  // The document before, until the one asked for is here: kept mounted (the
  // editor with it), out of reach, and gone from sight if the read is slow.
  const wait = !!open && !sameDoc(target, { source: open.source, name: open.id });
  return (
    <section className="ix-doc ix-in-main" data-testid="main-doc" data-doc={wait ? undefined : open?.id} data-wait={wait || undefined} hidden={!props.shown}>
      {notice && <DocNotice text={notice} dismiss={() => setNotice(null)} />}
      {open ? (
        <DocumentPane doc={doc} follow={props.follow} history={props.history} lead={props.lead} title={props.title} strips={props.strips} attach={props.attach} linkNote={props.linkNote}
          float={props.float && { title: asking ? (props.floatAs?.asking ?? "Opening it in the Ideas window…") : props.floatAs?.title ?? "Float: move it to the Ideas window", label: props.floatAs?.label, busy: asking, run: () => void float() }} />
      ) : (
        <header className="ix-head">
          <h1 className="ix-title">{failed ? "Not open" : "Opening…"}</h1>
          {failed && (
            <div className="ix-tools"><button className="ix-btn" data-testid="main-doc-again" onClick={() => setAgain((n) => n + 1)}>Try again</button></div>
          )}
        </header>
      )}
    </section>
  );
}
