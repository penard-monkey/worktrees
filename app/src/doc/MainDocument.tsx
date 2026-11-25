// A document open in the main window (workspace-chats.md §4.5, the documents
// lane): the shared pane under a plain header, with a way back to the
// workspace's overview and Float, which opens the same document in the Ideas
// window. Opened from the overview's plan and idea chips, behind Ideas' beta
// as the overview is.
//
// **Its text outlives its place on screen.** The main area shows something
// else the moment a place is picked in the nav, with nothing to ask first. So
// the pane stays mounted, hidden, until its text is saved, and only then goes:
// `shown` false is a save point, and a document whose text cannot be saved (a
// conflict, a removal) waits there with its banner until the person is back.
import { useEffect, useRef, useState } from "react";
import { invoke } from "../ipc";
import { DocNotice, DocumentPane } from "./DocumentPane";
import { ideaDocs, type DocRows } from "./source";
import { useDoc } from "./useDoc";

export type MainDocApi = {
  /** Save and say whether it may be left: false while its text is nowhere else. */
  leave(): Promise<boolean>;
};

export function MainDocument(props: {
  id: string;
  shown: boolean;
  /** Where Back goes, by name. */
  backTo: string;
  rows: DocRows;
  api: React.RefObject<MainDocApi | null>;
  /** Another document is the open one (a followed link, a merged id, a refusal). */
  opened(id: string): void;
  /** Saved, or never opened, and not on screen: let it go. */
  close(): void;
  /** Off screen with text that is not saved: it waits here. */
  waits(title: string): void;
  back(): void;
  float(id: string): void;
}) {
  const [notice, setNotice] = useState<string | null>(null);
  const [reading, setReading] = useState(true);
  const at = useRef(props);
  at.current = props;
  const doc = useDoc({
    source: ideaDocs,
    notice: setNotice,
    trace: (m) => { invoke("log_event", { level: "info", msg: `document: ${m}` }).catch(() => {}); },
    listed: () => {},
    rows: props.rows,
    follow: (id) => void go(id),
  });
  const { open, openDoc, leave, session, closeOpen } = doc;
  const openRef = useRef(open);
  openRef.current = open;

  /** Open one, and tell the window which it is now: a merged id lands on
   *  another, and a refused or failed open stays where it was. */
  const go = async (id: string) => {
    const ok = await openDoc(id);
    if (ok) at.current.opened(doc.current() ?? id);
    else { const cur = doc.current(); if (cur && cur !== at.current.id) at.current.opened(cur); }
  };
  useEffect(() => {
    let gone = false;
    setReading(true);
    void go(props.id).finally(() => { if (!gone) setReading(false); });
    return () => { gone = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the id asked for is the trigger
  }, [props.id]);

  useEffect(() => {
    const api = props.api;
    api.current = { leave };
    return () => { api.current = null; };
  }, [props.api, leave]);

  // Off screen: a save point, then gone, unless its text has nowhere to go.
  useEffect(() => {
    if (props.shown) return;
    let back = false;
    void leave().then((ok) => {
      if (back) return;
      if (ok) at.current.close();
      else at.current.waits(openRef.current?.title || "Untitled");
    });
    return () => { back = true; };
  }, [props.shown, leave]);

  // The last thing open was closed from its banner (it had been removed).
  const had = useRef(false);
  useEffect(() => {
    if (open) had.current = true;
    else if (had.current) at.current.back();
  }, [open]);

  const goBack = async () => { if (await leave()) { had.current = false; closeOpen(); props.back(); } };
  const float = async () => {
    const id = doc.current();
    if (!id) return;
    // The other window reads the file: what was typed here goes there first.
    await session.current?.flush();
    props.float(id);
  };
  const lead = (
    <button className="ix-btn" data-testid="main-doc-back" onClick={() => void goBack()} title={`Back to ${props.backTo}`}>‹ {props.backTo}</button>
  );
  return (
    <section className="ix-doc ix-in-main" data-testid="main-doc" data-doc={open?.id} hidden={!props.shown}>
      {notice && <DocNotice text={notice} dismiss={() => setNotice(null)} />}
      {open ? (
        <DocumentPane doc={doc} follow={(id) => void go(id)} lead={lead}
          after={<button className="ix-btn" data-testid="main-doc-float" onClick={() => void float()} title="Open it in the Ideas window">Float</button>} />
      ) : (
        <header className="ix-head">
          {lead}
          <h1 className="ix-title">{reading ? "Opening…" : "Not open"}</h1>
        </header>
      )}
    </section>
  );
}
