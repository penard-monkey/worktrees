// The Ideas window (docs/proposals/idea-board-phases.md §9.1, phase 1): the
// list of ideas, newest edited first, and one open idea in a CodeMirror
// editor with an Edit | Read switch.
//
// Every change the WINDOW makes (not the person's typing) is one named row
// with plain parameters — `idea_create`, `idea_write`, `idea_rename`,
// `idea_remove`, `idea_adopt` — so a later agent can propose the same call
// (§7). There is no second path that edits a document: the editor's text
// reaches the file only through `Session` (ideasave.ts) → `idea_write`.
//
// The document itself — its session, its editor, its versions, its header — is
// the pane both windows share (app/src/doc/: useDoc.ts, DocumentPane.tsx). What
// is here is this window's own: the list, the acts between documents, the plan
// panel and the conversations.
//
// Layout: a grid of [list | document | side]. The side column is empty and
// zero-wide in phase 1; the agent panel of phase 6 opens there, beside the
// editor, without moving either. List rows are plain elements carrying their
// id (`data-idea`), which is what phase 3's drag (navdrag.ts) will hang on.
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { engineState, invoke, listen, onEngineState, type EngineState } from "../src/ipc";
import { revealOnHost } from "../src/onhost";
import { wireMenu } from "../src/menu";
import * as Icons from "../src/icons";
import { WindowZoom } from "./zoom";
import { applySettings, loadSettings } from "../src/settings";
import { setTermScrollback } from "../src/TerminalPane";
import "../src/tokens.css";
import "../src/App.css";
import "./companion.css";
import { MemoryDoc, MemorySide, useMemoryView, type MemoryOpen } from "./MemoryDoc";
import { answer, ask, DOCK, FLOAT, take, windowEnv, type Ask, type HandDoc } from "../src/doc/handoff";
import type { MainDocApi } from "../src/doc/MainDocument";
import { DocNotice, DocumentPane } from "../src/doc/DocumentPane";
import { ideaDocs, type Doc, type Version } from "../src/doc/source";
import { errText, useDoc } from "../src/doc/useDoc";
import { quoteOf, type Quote } from "./ideaquote";
import type { ConvState } from "../src/desk";
import { IdeaChat, ideaDots, type IdeaSessionsEv } from "./IdeaChat";
import { useNavDrag } from "../src/navdrag";
import { CombineAsk, DropChooser, PickIdea, RelatedStrip, SplitAsk, type Relation } from "./IdeaRelate";
import {
  combineCall, combineDrop, ideaDrop, mergeCall, mergedNote, nextSelection,
  relateCall, splitCall, type Call, type Choice,
} from "../src/doc/ideasrelate";
import { PickProject, PickWorkspace, PlanPanel, PlanStrip, type RegProject, type WsOption } from "./PlanPanel";
import {
  addIdeasCall, addProjectCall, agentFor, fromPlanCall, ideaOntoPlan, itemPlace, launchCall, linkIdeaCall, linkPlaceCall, makePlanCall, MAIN_ASK_KEY, missingPlaces,
  placeOntoItem, planSources, plansIn, projectOntoPlan, removeIdeaCall, setPlanCall, startCall, WS_CHANGED_KEY, WS_SHOW_KEY,
  type Activity, type ItemRef, type MainAsk, type PlanView, type Snap, type StartDone, type WorkItem,
} from "../src/plans";
import { PickPlace, StartReview } from "../src/StartWork";
import { useGone } from "../src/useGone";

// ── the rows' shapes (worktrees_core::ideas) ────────────────────────────────
type Problem = "no-id" | "bad-id" | "duplicate-id" | "too-large" | "not-text" | "unreadable";
export type Idea = { id: string | null; title: string; file: string; created: string | null; mtime_ms: number; size: number; kind?: "idea" | "plan"; problem?: Problem };
type Listing = { ideas: Idea[]; skipped: string[]; dir?: string };

const PROBLEM: Record<Problem, string> = {
  "no-id": "Made by hand. Adopt it to open it here.",
  "bad-id": "Its id is not valid. Fix or remove the id line by hand.",
  "duplicate-id": "Another file carries the same id. Remove or re-id one copy by hand.",
  "too-large": "Too large to open (over 8 MB).",
  "not-text": "Not text (UTF-8), so it cannot be opened.",
  unreadable: "Could not be read.",
};

function useEngine(): EngineState {
  return useSyncExternalStore(onEngineState, engineState);
}

/** What a line in serve.log says about this window's connection: which socket
 *  (gen), from which origin, with which crypto. Read by a person after a
 *  hand test, since nothing may click this window for them. */
function connectLine(gen: number): string {
  const subtle = typeof window.crypto?.subtle?.verify === "function" ? "present" : "MISSING";
  return `companion: engine up (gen ${gen}) origin=${window.location.origin} crypto.subtle=${subtle}`;
}

/** A line in serve.log for a hand test to read back afterwards: an act and
 *  its outcome, never the document's text. gen 0 (the mock) has no log. */
function trace(gen: number, msg: string) {
  if (gen > 0) invoke("log_event", { level: "info", msg: `companion: ${msg}` }).catch(() => {});
}

/** Why the engine is not up, in words for the one muted line. */
function waitingWords(e: EngineState): string | null {
  switch (e.kind) {
    case "up":
      return null;
    case "starting":
      return "Connecting to the engine…";
    case "waiting":
    case "hidden":
      return "Waiting for the engine. The main window restarts it; this window reconnects on its own.";
    case "down":
      return `The engine is not running: ${e.reason}`;
  }
}

/** `2h`, `3d`: how long ago, for a list row. */
export function age(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 14) return `${Math.floor(s / 86400)}d`;
  return `${Math.floor(s / (86400 * 7))}w`;
}

/** No list yet: one array, so it is the same one every render. */
const NO_ROWS: Idea[] = [];

// ── the list ────────────────────────────────────────────────────────────────
function IdeaList(props: {
  listing: Listing | null;
  openId: string | null;
  filter: string;
  setFilter(f: string): void;
  onOpen(id: string): void;
  onNew(): void;
  onAdopt(i: Idea): void;
  onReveal(): void;
  canReveal: boolean;
  /** Each idea's conversations, at a glance (`ideas:sessions`). */
  dots: Map<string, ConvState>;
  /** Phase 3: the selected rows (⌘/Shift-click), and a press that may
   *  become a drag (navdrag's plumbing). */
  selected: string[];
  onSelect(id: string, mods: { toggle: boolean; range: boolean }, order: string[]): void;
  arm(e: React.PointerEvent, id: string): void;
  /** The row under a live drag that would take it, and whether "New idea"
   *  would (a drag of selected rows: Combine). */
  dropOn: string | null;
  newHot: boolean;
  onRowMenu(e: React.MouseEvent, i: Idea): void;
  onCombine(): void;
  /** Phase 4: how many selected ideas "Make a plan" would take, and it. */
  planFrom: number;
  onPlan(): void;
  /** Text dragged out of the editor over the list: Split (⌥ extracts). */
  textDrag: boolean;
  onTextDrop(extract: boolean): void;
  scrollRef: React.RefObject<HTMLUListElement | null>;
}) {
  const { listing, openId, filter } = props;
  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const all = [...(listing?.ideas ?? [])].sort((a, b) => b.mtime_ms - a.mtime_ms);
    return q ? all.filter((i) => i.title.toLowerCase().includes(q)) : all;
  }, [listing, filter]);
  const skipped = listing?.skipped ?? [];
  const order = shown.filter((i) => i.id && !i.problem).map((i) => i.id!);
  const [textHot, setTextHot] = useState(false);
  return (
    <nav className={`ix-list${textHot ? " ix-list-hot" : ""}`} aria-label="Ideas" data-testid="ideas-list"
      onDragOver={(e) => { if (props.textDrag) { e.preventDefault(); e.dataTransfer.dropEffect = e.altKey ? "copy" : "move"; setTextHot(true); } }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setTextHot(false); }}
      onDrop={(e) => { setTextHot(false); if (props.textDrag) { e.preventDefault(); props.onTextDrop(e.altKey); } }}>
      <div className="ix-list-head">
        <button className={`ix-btn ix-btn-primary${props.newHot ? " ix-drop" : ""}`} data-testid="ideas-new" data-drop-new="" onClick={props.onNew}
          title="A new idea. Drop two or more selected ideas here to combine them.">New idea</button>
        {props.selected.length >= 2 && (
          <button className="ix-btn" data-testid="ideas-combine" onClick={props.onCombine}>Combine {props.selected.length}…</button>
        )}
        {props.planFrom > 0 && props.selected.length >= 2 && (
          <button className="ix-btn" data-testid="ideas-plan-selected" onClick={props.onPlan}>Make a plan from {props.planFrom}</button>
        )}
        <input
          className="ix-filter"
          data-testid="ideas-filter"
          type="search"
          placeholder="Filter by title"
          value={filter}
          onChange={(e) => props.setFilter(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Escape") props.setFilter(""); }}
        />
      </div>
      {textHot && <p className="ix-quiet ix-list-tip">Drop to split into a new idea (⌥ keeps a copy here)</p>}
      <ul className="ix-rows" data-testid="ideas-rows" ref={props.scrollRef}>
        {shown.map((i) => {
          const key = i.id ?? `file:${i.file}`;
          const openable = i.id && !i.problem;
          return (
            <li
              key={key}
              className={`ix-row${i.id && i.id === openId ? " on" : ""}${i.problem ? " ix-row-problem" : ""}${i.id && props.selected.length > 1 && props.selected.includes(i.id) ? " ix-row-sel" : ""}${i.id && props.dropOn === i.id ? " ix-drop" : ""}`}
              data-idea={i.id ?? undefined}
              data-file={i.file}
              data-problem={i.problem}
              onPointerDown={openable ? (e) => props.arm(e, i.id!) : undefined}
              onContextMenu={openable ? (e) => props.onRowMenu(e, i) : undefined}
            >
              {openable ? (
                <button className="ix-row-main" data-drag-handle="" title={i.file}
                  onClick={(e) => {
                    const mods = { toggle: e.metaKey || e.ctrlKey, range: e.shiftKey };
                    if (mods.toggle || mods.range) props.onSelect(i.id!, mods, order);
                    else props.onOpen(i.id!);
                  }}>
                  {props.dots.get(i.id!) && <span className={`ix-dot ix-dot-${props.dots.get(i.id!)}`} data-testid="ideas-dot" data-state={props.dots.get(i.id!)}
                    aria-label={`A conversation is ${props.dots.get(i.id!) === "waiting" ? "waiting on you" : props.dots.get(i.id!) === "busy" ? "working" : "open"}`} />}
                  <span className="ix-row-title">{i.title || "Untitled"}</span>
                  {i.kind === "plan" && <span className="ix-row-tag" data-testid="ideas-plan-tag">plan</span>}
                  <span className="ix-row-age">{age(i.mtime_ms)}</span>
                </button>
              ) : (
                <div className="ix-row-main ix-row-static" title={i.file}>
                  <span className="ix-row-title">{i.title || i.file}</span>
                  <span className="ix-row-age">{age(i.mtime_ms)}</span>
                </div>
              )}
              {i.problem && (
                <div className="ix-row-why">
                  <span>{PROBLEM[i.problem]}</span>
                  {i.problem === "no-id" && (
                    <button className="ix-btn ix-btn-small" data-testid="ideas-adopt" onClick={() => props.onAdopt(i)}>Adopt</button>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {listing && shown.length === 0 && (
        <p className="ix-quiet ix-list-empty">{filter.trim() ? "No idea's title matches." : "No ideas yet."}</p>
      )}
      <div className="ix-list-foot">
        {skipped.length > 0 && (
          <p className="ix-quiet" data-testid="ideas-skipped" title={skipped.join("\n")}>
            {skipped.length === 1 ? "1 entry in the folder is not a document" : `${skipped.length} entries in the folder are not documents`}: {skipped.join(", ")}
          </p>
        )}
        {props.canReveal && <button className="ix-btn ix-btn-quiet" data-testid="ideas-reveal-folder" onClick={props.onReveal}>Show in Finder</button>}
      </div>
    </nav>
  );
}

// ── the window ──────────────────────────────────────────────────────────────
const zoom = new WindowZoom();
const zoomReady = zoom.load();

export function Companion() {
  const engine = useEngine();
  const up = engine.kind === "up";
  const gen = engine.gen;
  const [listing, setListing] = useState<Listing | null>(null);
  const [filter, setFilter] = useState("");
  const [notice, setNoticeState] = useState<string | null>(null);
  /** The last thing said, for an answer to the other window. */
  const lastNotice = useRef<string | null>(null);
  const memStuck = useRef<string | null>(null);
  const setNotice = useCallback((m: string | null) => { if (m !== null) lastNotice.current = m; setNoticeState(m); }, []);
  // Phase S: the open idea's conversations beside the editor.
  const [sessions, setSessions] = useState<IdeaSessionsEv | null>(null);
  const [chatOpen, setChatOpen] = useState(false);
  const [prefs, setPrefs] = useState<{ terminal: string; uiRem: number }>({ terminal: "", uiRem: 15 });
  const dots = useMemo(() => ideaDots(sessions), [sessions]);
  // Phase 3: selection, drags, the asks, the `[[` picker, history, relations.
  const [selected, setSelected] = useState<string[]>([]);
  const [chooser, setChooser] = useState<{ from: string; onto: string; x: number; y: number } | null>(null);
  const [pick, setPick] = useState<{ act: "relate" | "merge"; id: string } | null>(null);
  const [combineAsk, setCombineAsk] = useState<string[] | null>(null);
  const [splitAsk, setSplitAsk] = useState<{ start: number; end: number; text: string } | null>(null);
  const [rowMenu, setRowMenu] = useState<{ idea: Idea; x: number; y: number } | null>(null);
  const [undo, setUndo] = useState<{ label: string; run: () => Promise<void> } | null>(null);
  const [related, setRelated] = useState<Relation[]>([]);
  // The selection as a message would quote it, read while a conversation's
  // panel is open.
  const [quote, setQuote] = useState<Quote | null>(null);
  const quoteTimer = useRef<number | undefined>(undefined);
  const textDragging = useRef(false);
  const [textDrag, setTextDrag] = useState(false);
  const listRef = useRef<HTMLUListElement | null>(null);
  const lastXY = useRef({ x: 0, y: 0 });
  // Phase 4: the open plan as core's reader answers it, and the pickers.
  const [planView, setPlanView] = useState<PlanView | null>(null);
  const [regProjects, setRegProjects] = useState<RegProject[] | null>(null);
  const [wsPick, setWsPick] = useState<{ mode: "link-idea" | "set-plan"; id: string; rows: WsOption[] } | null>(null);
  const [addIdeaPick, setAddIdeaPick] = useState<string | null>(null);
  // Phase 5: the snapshot the nav reads (asked for while a plan is open), who
  // works, the item under review, and the item a place is picked for.
  const [snap, setSnap] = useState<Snap>(null);
  const [activity, setActivity] = useState<Activity>({ busy: new Set(), waiting: new Set() });
  const [placesTick, setPlacesTick] = useState(0);
  const [reviewing, setReviewing] = useState<WorkItem | null>(null);
  const [placeFor, setPlaceFor] = useState<WorkItem | null>(null);
  const [agentPrefs, setAgentPrefs] = useState<{ provider: string; models: Record<string, string | undefined> }>({ provider: "claude", models: {} });

  // Once per socket: the window's theme from the shared settings (read only —
  // the main window owns ui-state.json's whole-blob save), and the connect
  // line. gen 0 is a door with no engine (the mock): nothing to log to.
  useEffect(() => {
    if (!up) return;
    // `app_zoom` too, for a window with no size of its own (zoom.ts).
    Promise.all([loadSettings(), zoomReady]).then(([st]) => { applySettings(st); zoom.inherit(st.app_zoom); setTermScrollback(st.term_scrollback); setPrefs({ terminal: st.terminal_cmd, uiRem: st.ui_rem }); setAgentPrefs({ provider: st.default_provider, models: st.default_models }); }).catch(() => {});
    if (gen > 0) invoke("log_event", { level: "info", msg: connectLine(gen) }).catch(() => {});
  }, [up, gen]);

  const refresh = useCallback(async () => {
    try { setListing(await invoke<Listing>("ideas_list")); } catch (e) { setNotice(`Could not list the ideas: ${errText(e)}`); }
  }, []);
  useEffect(() => { if (up) void refresh(); }, [up, gen, refresh]);

  // The open document. An edit anywhere in the folder re-pulls the list,
  // whatever the event carries (a hand-made file has no id to name), and bumps
  // `relTick`; the open document's session decides what it means for the editor.
  const doc = useDoc({
    source: ideaDocs,
    notice: setNotice,
    trace: (m) => trace(gen, m),
    listed: () => void refresh(),
    rows: listing?.ideas ?? NO_ROWS,
    follow: (to) => void openAny(to),
    selectionMoved: () => readQuote(true),
    // The same offsets over another text: offered, never attached by itself.
    replaced: () => readQuote(false),
    began: () => { window.clearTimeout(quoteTimer.current); setQuote(null); },
  });
  const { open, mode, busy, setBusy, session, handle, arm, hasSel, tick: relTick, bump, begin, leave: leaveIdea, closeOpen, openDoc, setMode, gotoLine } = doc;
  // A workspace's memory file, floated here (MemoryDoc.tsx). One document is open in
  // this window: an idea, through `doc`, or that one; opening either leaves the other.
  const [mem, setMem] = useState<MemoryOpen | null>(null);
  const memApi = useRef<MainDocApi | null>(null);
  const memLanded = useRef<((ok: boolean) => void) | null>(null);
  const memRef = useRef(mem);
  memRef.current = mem;
  const memView = useMemoryView(up && mem ? mem.ws : null);
  const leave = useCallback(async (): Promise<boolean> => {
    if (memRef.current && memApi.current && !(await memApi.current.leave())) return false;
    if (!(await leaveIdea())) return false;
    setMem(null);
    return true;
  }, [leaveIdea]);
  const openIdea = useCallback(async (id: string): Promise<boolean> => {
    const was = memRef.current;
    if (was && memApi.current && !(await memApi.current.leave())) return false;
    if (memRef.current !== was) return false;
    const ok = await openDoc(id);
    if (ok) setMem(null);
    return ok;
  }, [openDoc]);
  /** Open a memory file here. Resolved once it is open, or it is not. */
  const openMemory = useCallback(async (to: MemoryOpen): Promise<boolean> => {
    if (!memRef.current && !(await leaveIdea())) return false;
    if (memRef.current && memApi.current && (memRef.current.ws !== to.ws || memRef.current.path !== to.path) && !(await memApi.current.leave())) return false;
    if (memRef.current?.ws === to.ws && memRef.current.path === to.path) { setMem(to); return true; }
    closeOpen();
    memLanded.current?.(false);
    return new Promise((done) => {
      memLanded.current = (ok) => { memLanded.current = null; done(ok); };
      setMem(to);
    });
  }, [leaveIdea, closeOpen]);
  const landed = useCallback((path: string, ok: boolean) => { if (memRef.current?.path === path) memLanded.current?.(ok); }, []);
  const openAny = (to: { name: string; source: unknown }) => (to.source === ideaDocs ? openIdea(to.name) : Promise.resolve(false));
  /** As the list's rules take it (an idea or a plan: this window's documents). */
  const openRow = open && { id: open.id, kind: open.kind === "plan" ? ("plan" as const) : ("idea" as const) };

  // The conversations' live state, by tmux session (serve sends the last one
  // as soon as this window declares the event).
  useEffect(() => {
    let off: (() => void) | null = null;
    let gone = false;
    listen<IdeaSessionsEv>("ideas:sessions", (ev) => { if (ev.payload) setSessions(ev.payload); })
      .then((u) => { if (gone) u(); else off = u; }).catch(() => {});
    return () => { gone = true; off?.(); };
  }, []);

  const newIdea = useCallback(async () => {
    if (!(await leave())) return;
    try {
      const d = await invoke<Doc>("idea_create", { title: null });
      trace(gen, `created ${d.id}`);
      void refresh();
      // Typed into the one before while it was made.
      if (!(await leave())) return;
      setMode("edit");
      begin(d, true);
    } catch (e) { setNotice(`Could not make a new idea: ${errText(e)}`); }
  }, [leave, begin, setMode, refresh, gen]);

  const adopt = useCallback(async (i: Idea) => {
    if (!(await leave())) return;
    try {
      const d = await invoke<Doc>("idea_adopt", { file: i.file, expectedMtime: i.mtime_ms });
      trace(gen, `adopted ${i.file} as ${d.id}`);
      void refresh();
      if (!(await leave())) return;
      begin(d);
    } catch (e) { setNotice(`Could not adopt ${i.file}: ${errText(e)}`); void refresh(); }
  }, [leave, begin, refresh, gen]);

  const remove = useCallback(() => arm.confirm("remove", async () => {
    const s = session.current;
    if (!s || !open) return;
    if (!(await s.drain())) { setNotice("Save or reload the open idea first: its text is not saved."); return; }
    setBusy(true);
    try {
      await invoke("idea_remove", { id: open.id, expectedMtime: s.lastMtime });
      trace(gen, `removed ${open.id}`);
      closeOpen();
      setNotice(`Removed “${open.title}”. Its text is kept in the folder's .history.`);
      void refresh();
    } catch (e) { setNotice(`Could not remove it: ${errText(e)}`); }
    finally { setBusy(false); }
  }), [arm, open, session, setBusy, closeOpen, refresh, gen]);

  // ── phase 3: the acts (§3, §6.1). Every path, drag or menu, ends in `act`
  // with one row call from ideasrelate.ts. The open idea is saved first and
  // refused while it cannot be (its text exists nowhere else); afterwards
  // the open idea is read again, through the index, so a merged-away one
  // lands on its absorber with the note.
  const act = useCallback(async <T,>(call: Call, label: string): Promise<T | null> => {
    const s = session.current;
    if (s) {
      if (!(await s.drain()) || s.held() || s.current.kind === "error") { setNotice("Save or reload the open idea first: its text is not saved."); return null; }
    }
    setBusy(true);
    try {
      const out = await invoke<T>(call.cmd, call.args);
      trace(gen, `${call.cmd} ${label}`);
      void refresh();
      setSelected([]);
      return out;
    } catch (e) { setNotice(`Could not ${label}: ${errText(e)}`); void refresh(); return null; }
    finally { setBusy(false); }
  }, [gen, refresh, session, setBusy]);

  /** Read the open idea again after an act changed or absorbed it. */
  const reopen = useCallback(async (d?: Doc) => {
    const cur = open?.id;
    if (!cur) return;
    try {
      const doc = d ?? await invoke<Doc>("idea_read", { id: cur });
      begin(doc);
      if (doc.merged) setNotice(mergedNote(doc.merged, doc.title));
    } catch { closeOpen(); }
  }, [open, begin, closeOpen]);

  /** Undo by history: put each idea's newest copy back, remove the new one. */
  const undoBy = useCallback((restore: string[], remove: string[]) => async () => {
    setUndo(null);
    try {
      for (const id of remove) await invoke("idea_remove", { id });
      for (const id of restore) {
        const v = await invoke<Version[]>("idea_versions", { id });
        if (v[0]) await invoke("idea_restore", { id, version: v[0].version });
      }
      trace(gen, `undo: restored ${restore.join(",")} removed ${remove.join(",")}`);
      setNotice("Undone. Every version is still in History.");
      void refresh();
      if (open && (restore.includes(open.id) || remove.includes(open.id))) {
        if (remove.includes(open.id) && restore.length) { await openIdeaRef.current(restore[0]); } else await reopen();
      }
    } catch (e) { setNotice(`Could not undo: ${errText(e)}`); }
  }, [gen, refresh, open, reopen]);
  const openIdeaRef = useRef(openIdea);
  openIdeaRef.current = openIdea;

  const titleOf = useCallback((id: string) => listing?.ideas.find((i) => i.id === id)?.title ?? "Untitled", [listing]);
  const mtimeOf = (id: string) => (open?.id === id ? session.current?.lastMtime : undefined);

  const doRelate = useCallback(async (id: string, other: string) => {
    const d = await act<Doc>(relateCall(id, other), `relate ${id} to ${other}`);
    if (!d) return;
    setNotice(`Related “${titleOf(id)}” to “${titleOf(other)}”. Undo with History.`);
    if (open?.id === id) await reopen(d); else bump();
  }, [act, titleOf, open, reopen]);

  /** `from` merged into `into`: asked first (the chooser or the dialog),
   *  and Undo offered after. */
  const doMerge = useCallback(async (into: string, from: string) => {
    const [ti, tf] = [titleOf(into), titleOf(from)];
    const d = await act<Doc>(mergeCall(into, from, { into: mtimeOf(into), from: mtimeOf(from) }), `merge ${from} into ${into}`);
    if (!d) return;
    setNotice(`Merged “${tf}” into “${ti}”.`);
    setUndo({ label: "Undo merge", run: undoBy([from, into], []) });
    if (open?.id === into || open?.id === from) {
      if (open.id !== d.id && !(await leave())) return;
      begin(d);
      if (open?.id === from) setNotice(`Merged “${tf}” into “${ti}”: this is “${ti}” now.`);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [act, titleOf, undoBy, open, begin]);

  const doCombine = useCallback(async (ids: string[], title: string) => {
    setCombineAsk(null);
    const d = await act<Doc>(combineCall(ids, title), `combine ${ids.join(",")}`);
    if (!d) return;
    setNotice(`Combined ${ids.length} ideas into “${d.title}”.`);
    setUndo({ label: "Undo combine", run: undoBy(ids, [d.id!]) });
    if (await leave()) begin(d);
  }, [act, undoBy, leave, begin]);

  const doSplit = useCallback(async (sel: { start: number; end: number }, extract: boolean, title?: string) => {
    if (!open || !session.current) return;
    setSplitAsk(null);
    await session.current.drain();
    const id = open.id;
    const out = await act<{ new: Doc; source: Doc }>(splitCall(id, sel.start, sel.end, extract, session.current.lastMtime, title), `${extract ? "extract" : "split"} from ${id}`);
    if (!out) return;
    setNotice(`${extract ? "Extracted" : "Split"} “${out.new.title}” into a new idea.${extract ? "" : " A link to it takes the text's place."}`);
    setUndo({ label: `Undo ${extract ? "extract" : "split"}`, run: undoBy(extract ? [] : [id], [out.new.id!]) });
    if (!extract) begin(out.source); else bump();
  }, [open, act, undoBy, begin]);

  // ── phase 4: plans (§2.2, §6.1). Each act is one call from plans.ts,
  // through `act` (the open document saved first — and before the call is
  // built, since an edit to the open plan carries its mtime AFTER that save). A change to the OPEN
  // plan comes back as its new text and is loaded, so the editor never
  // holds the text the act replaced (the act copied that to history).
  const ideaRowsAll = listing?.ideas ?? [];
  const rowOf = (id: string) => ideaRowsAll.find((i) => i.id === id) ?? null;
  const afterPlanEdit = useCallback(async (d: Doc | null) => {
    if (!d) return;
    if (open?.id === d.id) await reopen(d); else bump();
  }, [open, reopen]);

  const doMakePlan = useCallback(async (ids: string[]) => {
    if (ids.length === 0) { setNotice("Pick an idea to make a plan from."); return; }
    const d = await act<Doc>(makePlanCall(ids), `make a plan from ${ids.join(",")}`);
    if (!d) return;
    setNotice(`Made “${d.title}” from ${ids.length === 1 ? `“${titleOf(ids[0])}”` : `${ids.length} ideas`}. Fill in its Goal, Projects and Work.`);
    if (await leave()) begin(d);
  }, [act, titleOf, leave, begin]);

  const doAddIdeas = useCallback(async (plan: string, ids: string[]) => {
    await session.current?.drain();
    const d = await act<Doc>(addIdeasCall(plan, ids, mtimeOf(plan)), `add ${ids.join(",")} to plan ${plan}`);
    if (!d) return;
    setNotice(`Added ${ids.length === 1 ? `“${titleOf(ids[0])}”` : `${ids.length} ideas`} to “${d.title}”.`);
    await afterPlanEdit(d);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [act, titleOf, afterPlanEdit]);

  const doRemoveIdea = useCallback(async (plan: string, idea: string) => {
    await session.current?.drain();
    const d = await act<Doc>(removeIdeaCall(plan, idea, mtimeOf(plan)), `take ${idea} out of plan ${plan}`);
    if (!d) return;
    setNotice(`Took it out of “${d.title}”. The idea stays; the plan's last version is in History.`);
    await afterPlanEdit(d);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [act, afterPlanEdit]);

  const doAddProject = useCallback(async (plan: string, root: string) => {
    await session.current?.drain();
    const d = await act<Doc>(addProjectCall(plan, root, mtimeOf(plan)), `add project ${root} to plan ${plan}`);
    if (!d) return;
    setNotice(`Added the project to “${d.title}”'s Projects.`);
    await afterPlanEdit(d);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [act, afterPlanEdit]);

  /** Show a workspace in the main window: the hand-off key, then raise it. */
  const showWorkspace = useCallback((id: string) => {
    try { window.localStorage.setItem(WS_SHOW_KEY, JSON.stringify({ id, at: Date.now() })); } catch { /* no storage: the window still comes forward */ }
    invoke("raise_window").catch(() => {});
  }, []);

  /** A workspace was changed here: the main window reads its list again. */
  const wsChanged = () => { try { window.localStorage.setItem(WS_CHANGED_KEY, String(Date.now())); } catch { /* it reads on focus */ } };
  const doMakeWorkspace = useCallback(async (plan: string) => {
    const out = await act<{ id: string; created: boolean; workspaces: { id: string; name: string }[] }>(fromPlanCall(plan), `make workspace from ${plan}`);
    if (!out) return;
    wsChanged();
    const name = out.workspaces.find((w) => w.id === out.id)?.name ?? "";
    setNotice(out.created ? `Made the workspace “${name}”. It is in the main window, under Workspaces.` : `This plan already has a workspace, “${name}”: showing it.`);
    bump();
    showWorkspace(out.id);
  }, [act, showWorkspace]);

  const openWsPick = useCallback(async (mode: "link-idea" | "set-plan", id: string) => {
    try {
      const l = await invoke<{ workspaces: WsOption[] }>("workspaces_list");
      const rows = l.workspaces.filter((w) => (mode === "set-plan" ? !w.plan : !w.ideas.includes(id)));
      setWsPick({ mode, id, rows });
    } catch (e) { setNotice(`Could not list the workspaces: ${errText(e)}`); }
  }, []);
  const doWsPick = useCallback(async (ws: string) => {
    const p = wsPick;
    setWsPick(null);
    if (!p) return;
    const call = p.mode === "set-plan" ? setPlanCall(ws, p.id) : linkIdeaCall(ws, p.id);
    const l = await act<{ workspaces: { id: string; name: string }[] }>(call, `${p.mode} ${p.id} → ${ws}`);
    if (!l) return;
    wsChanged();
    const name = l.workspaces.find((w) => w.id === ws)?.name ?? "";
    setNotice(p.mode === "set-plan" ? `“${name}” is this plan's workspace now.` : `Linked “${titleOf(p.id)}” to “${name}”.`);
    bump();
  }, [wsPick, act, titleOf]);

  const openProjectPick = useCallback(async () => {
    try {
      const st = await invoke<{ projects: RegProject[] }>("cross_project_status");
      setRegProjects(st.projects.map((p) => ({ root: p.root, name: p.name })));
    } catch (e) { setNotice(`Could not list the projects: ${errText(e)}`); }
  }, []);

  // The open plan, read through core's reader: again on any folder change.
  useEffect(() => {
    if (!open || open.kind !== "plan" || !up) { setPlanView(null); return; }
    let gone = false;
    invoke<PlanView>("plan_read", { id: open.id }).then((v) => { if (!gone) setPlanView(v); })
      .catch((e) => { if (!gone) { setPlanView(null); setNotice(`Could not read the plan: ${errText(e)}`); } });
    return () => { gone = true; };
  }, [open?.id, open?.kind, open?.key, relTick, up]);

  // ── phase 5: what a plan does (§2.4). Clone and Create are the main
  // window's own dialogs, asked for by key; Start and Link a place are rows.
  const planOpen = !!open && open.kind === "plan" && up;
  // Two lists may be in flight; one asked for earlier never replaces a later's.
  const listSeq = useRef({ asked: 0, shown: 0 });
  const listPlaces = useCallback(async () => {
    const n = ++listSeq.current.asked;
    const w = await invoke<Snap>("list_workspace");
    if (n < listSeq.current.shown) return;
    listSeq.current.shown = n;
    setSnap(w);
  }, []);
  useEffect(() => {
    if (!planOpen) { listSeq.current.shown = ++listSeq.current.asked; setSnap(null); return; }
    listPlaces().catch((e) => setNotice(`Could not read the places: ${errText(e)}`));
  }, [planOpen, open?.id, placesTick, gen, listPlaces]);
  useEffect(() => {
    if (!planOpen) return;
    const offs = [
      listen("places:changed", () => setPlacesTick((n) => n + 1)),
      listen<{ busy: string[]; waiting: string[] }>("sessions:busy", (e) => { if (e.payload) setActivity({ busy: new Set(e.payload.busy), waiting: new Set(e.payload.waiting) }); }),
    ];
    return () => { for (const u of offs) u.then((f) => f()).catch(() => {}); };
  }, [planOpen]);
  const gonePlaces = useGone(
    useMemo(() => (planView ? missingPlaces(planView.work, planView.work_states, snap) : []), [planView, snap]),
    listPlaces,
  );
  useEffect(() => { setReviewing(null); setPlaceFor(null); }, [open?.id]);
  const askMain = useCallback((a: MainAsk) => {
    try { window.localStorage.setItem(MAIN_ASK_KEY, JSON.stringify({ ...a, at: Date.now() })); } catch { setNotice("Could not reach the main window from here."); return; }
    invoke("raise_window").catch(() => {});
  }, []);
  const bringProject = useCallback(async (plan: string, name: string) => {
    await session.current?.drain();
    trace(gen, `asked the main window for ${name} of plan ${plan}`);
    askMain({ act: "project", plan, name });
  }, [askMain, gen]);
  const doLinkPlace = useCallback(async (plan: string, item: ItemRef, root: string, slug: string) => {
    await session.current?.drain();
    const d = await act<Doc>(linkPlaceCall(plan, item, root, slug, mtimeOf(plan)), `link ${slug} to “${item.title}”`);
    if (!d) return;
    setNotice(`“${item.title}” has the place ${slug} now.`);
    await afterPlanEdit(d);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [act, afterPlanEdit]);
  /** Start one reviewed item. Its refusals belong in the review, so they
   *  reach it as they are rather than through `act`'s notice. */
  const doStart = useCallback(async (plan: string, item: WorkItem, root: string, brief: string): Promise<StartDone> => {
    const s = session.current;
    if (s) {
      if (!(await s.drain()) || s.held() || s.current.kind === "error") throw "Save or reload the plan first: its text is not saved.";
    }
    const c = startCall(plan, item, root, brief, agentFor(item, agentPrefs), mtimeOf(plan));
    const out = await invoke<StartDone>(c.cmd, c.args);
    trace(gen, `plan_start ${plan} “${item.title}”: ${out.started ? `place ${out.started.slug}` : "no place"} rc=${out.result.code}`);
    if (out.doc) await afterPlanEdit(out.doc as Doc);
    setPlacesTick((n) => n + 1);
    void refresh();
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [afterPlanEdit, agentPrefs, gen, refresh]);

  // The main window asks for a document (Float): opened here, then answered by
  // name. It keeps its own until then, so a refusal here is said there.
  const showAsked = useRef(async (_a: Ask) => {});
  showAsked.current = async (a) => {
    const d = a.doc;
    trace(gen, `asked to show ${d.kind === "idea" ? d.id : `${d.ws} ${d.path}`}`);
    lastNotice.current = null;
    const ok = d.kind === "idea" ? await openIdea(d.id) : await openMemory({ ws: d.ws, name: d.name, path: d.path });
    const at = memRef.current;
    const now: HandDoc | null = d.kind === "idea" ? d : at ? { kind: "memory", ...at } : d;
    answer(windowEnv(), FLOAT, a, ok && now ? { ok: true, doc: d.kind === "memory" ? d : now } : { ok: false, why: unsavedHere.current() ?? `The Ideas window did not open it${lastNotice.current ? `: ${lastNotice.current}` : "."}` });
  };
  /** What is open here with text that is not saved. */
  const unsavedHere = useRef((): string | null => null);
  unsavedHere.current = () => (session.current?.dirty() && open ? `The Ideas window has “${open.title || "Untitled"}” open with text that is not saved.` : memRef.current && memStuck.current ? `The Ideas window has “${memStuck.current}” open with text that is not saved.` : null);
  useEffect(() => {
    if (!up) return;
    const look = () => { const a = take(windowEnv(), FLOAT); if (a) void showAsked.current(a); };
    const onStorage = (e: StorageEvent) => { if (e.key === FLOAT.ask) look(); };
    look();
    window.addEventListener("focus", look);
    window.addEventListener("storage", onStorage);
    return () => { window.removeEventListener("focus", look); window.removeEventListener("storage", onStorage); };
  }, [up, gen]);

  // The related ideas follow the open idea, and any change in the folder.
  useEffect(() => {
    if (!open || !up) { setRelated([]); return; }
    let gone = false;
    invoke<Relation[]>("idea_related", { id: open.id }).then((r) => { if (!gone) setRelated(r); }).catch(() => { if (!gone) setRelated([]); });
    return () => { gone = true; };
  }, [open?.id, relTick, up]);

  // ── the drag: one idea onto another (the chooser), or the selected rows
  // onto "New idea" (Combine). navdrag's plumbing; Escape cancels there.
  type IdeaDrag = { kind: "idea"; id: string } | { kind: "ideas"; ids: string[] } | { kind: "project"; root: string } | { kind: "place"; root: string; slug: string };
  type IdeaTarget = { kind: "work"; plan: string; item: ItemRef; root: string; slug: string } | { kind: "onto"; id: string } | { kind: "new"; ids: string[] } | { kind: "plan"; id: string; ids: string[] } | { kind: "plan-projects"; id: string; root: string };
  const ideaRows = listing?.ideas ?? [];
  const { drag, arm: armDrag } = useNavDrag<IdeaTarget, IdeaDrag>({
    scrollRef: listRef,
    resolve: (item, x, y) => {
      lastXY.current = { x, y };
      const el = document.elementFromPoint(x, y) as HTMLElement | null;
      // Phase 4: a registered project (from Add project…) onto the open
      // plan's Projects box.
      if (item.kind === "project") {
        const box = el?.closest<HTMLElement>("[data-drop-plan-projects]");
        return box && planView && box.dataset.dropPlanProjects === planView.doc.id && projectOntoPlan(item.root, planView)
          ? { kind: "plan-projects", id: planView.doc.id, root: item.root } : null;
      }
      // Phase 5: a place (from Link a place…) onto one of the plan's items.
      if (item.kind === "place") {
        const box = el?.closest<HTMLElement>("[data-drop-work]");
        const n = box && planView?.work ? planView.work.findIndex((w) => w.line === Number(box.dataset.dropWork)) : -1;
        if (n < 0 || !planView) return null;
        const w = planView.work![n];
        const ref = placeOntoItem(item.root, item.slug, w, itemPlace(w, planView.work_states[n], snap, activity, gonePlaces), snap);
        return ref ? { kind: "work", plan: planView.doc.id, item: ref, root: item.root, slug: item.slug } : null;
      }
      // An idea (or the selected ones) onto a plan: the open plan's strip,
      // or a plan's row in the list. Adds; never the relate/merge chooser.
      const ids = item.kind === "ideas" ? item.ids : [item.id];
      const strip = el?.closest<HTMLElement>("[data-drop-plan]");
      const planRow = el?.closest<HTMLElement>("[data-idea]");
      const planId = strip?.dataset.dropPlan ?? (planRow && rowOf(planRow.dataset.idea ?? "")?.kind === "plan" ? planRow.dataset.idea! : null);
      if (planId) {
        const planned = planView?.doc.id === planId ? planView.ideas.map((i) => i.live ?? i.id) : [];
        const ok = ids.filter((i) => ideaOntoPlan(rowOf(i), rowOf(planId) ?? { id: planId, kind: "plan" }, planned) === planId);
        return ok.length ? { kind: "plan", id: planId, ids: ok } : null;
      }
      if (item.kind === "ideas") {
        const c = el?.closest("[data-drop-new]") ? combineDrop(item.ids) : null;
        return c ? { kind: "new", ids: c } : null;
      }
      // Relate / merge are between ideas: a plan is neither side of them.
      if (rowOf(item.id)?.kind === "plan") return null;
      const row = el?.closest<HTMLElement>("[data-idea]");
      const ontoRow = row ? ideaRows.find((i) => i.id === row.dataset.idea) ?? null : null;
      const onto = ontoRow && ontoRow.kind !== "plan" ? ideaDrop(item.id, ontoRow) : null;
      return onto ? { kind: "onto", id: onto } : null;
    },
    commit: (item, target) => {
      if (!target) return;
      if (target.kind === "plan") { void doAddIdeas(target.id, target.ids); return; }
      if (target.kind === "plan-projects") { void doAddProject(target.id, target.root); return; }
      if (target.kind === "work") { setPlaceFor(null); void doLinkPlace(target.plan, target.item, target.root, target.slug); return; }
      if (target.kind === "new") { setCombineAsk(target.ids); return; }
      if (item.kind === "idea") setChooser({ from: item.id, onto: target.id, ...lastXY.current });
    },
  });
  const armRow = (e: React.PointerEvent, id: string) => {
    const many = selected.length >= 2 && selected.includes(id);
    armDrag(e, many ? { kind: "ideas", ids: selected } : { kind: "idea", id });
  };
  const choose = (c: Choice | null) => {
    const ch = chooser;
    setChooser(null);
    if (!ch || !c) return;
    if (c === "relate") void doRelate(ch.from, ch.onto);
    else void doMerge(ch.onto, ch.from);
  };

  /** Read the selection into `quote`, once it has held still: a drag over a
   *  long document moves it every frame. None is at once. */
  const quoting = !!open && chatOpen && mode === "edit";
  const readQuote = (auto: boolean, wait = 120) => {
    window.clearTimeout(quoteTimer.current);
    if (!quoting || !handle.current?.selection()) { setQuote(null); return; }
    quoteTimer.current = window.setTimeout(() => {
      const [h, sel] = [handle.current, handle.current?.selection()];
      setQuote(h && sel && open ? quoteOf(open.title, h.text(), sel, auto) : null);
    }, wait);
  };
  useEffect(() => {
    readQuote(true, 0);
    return () => window.clearTimeout(quoteTimer.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the panel opening, the view, or the idea is the trigger
  }, [quoting, open?.key]);
  const textDragOn = (on: boolean) => { textDragging.current = on; setTextDrag(on); };
  const onTextDrop = (extract: boolean) => {
    textDragOn(false);
    const sel = handle.current?.selection();
    if (sel) void doSplit(sel, extract);
  };
  const askSplit = () => {
    const sel = handle.current?.selection();
    if (!sel) { setNotice("Select the text to split out first."); return; }
    setSplitAsk(sel);
  };


  // This window's own size chords, and the menu bar's items for it.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { zoom.key(e); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const menuAct = useRef<(id: string) => void>(() => {});
  menuAct.current = (id) => { if (id === "new-idea") void newIdea(); };
  useEffect(() => wireMenu("companion", (id) => menuAct.current(id)), []);

  const words = waitingWords(engine);
  const reveal = () => {
    if (!listing?.dir) return;
    revealOnHost(open ? `${listing.dir}/${open.file}` : listing.dir).catch((e) => setNotice(`Could not show it in Finder: ${errText(e)}`));
  };

  return (
    <main className={`ix${open && chatOpen ? " ix-with-chat" : ""}`}>
      {mem ? (
        <MemorySide at={mem} view={memView.view} error={memView.error} ago={(at) => age(at * 1000)} open={(path) => void openMemory({ ...mem, path })} all={() => void leave()} />
      ) : <IdeaList
        listing={listing}
        openId={open?.id ?? null}
        filter={filter}
        setFilter={setFilter}
        onOpen={(id) => { setSelected([]); void openIdea(id); }}
        onNew={() => void newIdea()}
        onAdopt={(i) => void adopt(i)}
        onReveal={reveal}
        canReveal={!!listing?.dir}
        dots={dots}
        selected={selected}
        onSelect={(id, mods, order) => setSelected((cur) => nextSelection(cur.length ? cur : open ? [open.id] : [], id, mods, order, cur[cur.length - 1] ?? open?.id ?? null))}
        arm={armRow}
        dropOn={drag?.target?.kind === "onto" ? drag.target.id : null}
        newHot={drag?.target?.kind === "new"}
        onRowMenu={(e, i) => { e.preventDefault(); setRowMenu({ idea: i, x: e.clientX, y: e.clientY }); }}
        onCombine={() => setCombineAsk(selected)}
        planFrom={planSources(null, selected, ideaRowsAll).length}
        onPlan={() => void doMakePlan(planSources(null, selected, ideaRowsAll))}
        textDrag={textDrag}
        onTextDrop={onTextDrop}
        scrollRef={listRef}
      />}
      <section className="ix-doc" data-testid="ideas-doc">
        {words && <p className="ix-engine" data-testid="companion-engine">{words}</p>}
        {notice && (
          <DocNotice text={notice} dismiss={() => { setNotice(null); setUndo(null); }}
            act={undo && <button className="ix-btn" data-testid="ideas-undo" onClick={() => void undo.run()}>{undo.label}</button>} />
        )}
        {mem ? (
          <MemoryDoc at={mem} view={memView.view} tick={memView.tick} api={memApi} landed={landed}
            go={(path) => setMem((m) => m && { ...m, path })}
            follow={(path) => void openMemory({ ...mem, path })}
            idea={(id) => void openIdea(id)}
            stuck={(t) => { memStuck.current = t; }}
            dock={(path) => ask(windowEnv(), DOCK, { kind: "memory", ws: mem.ws, name: mem.name, path }, () => invoke("raise_window"), "The main window")}
            docked={() => setMem(null)} closed={() => setMem(null)}
            reveal={(p) => { revealOnHost(p).catch((e) => setNotice(`Could not show it in Finder: ${errText(e)}`)); }} />
        ) : open ? (
          <DocumentPane
            doc={doc}
            follow={(to) => void openAny(to)}
            textDrag={textDragOn}
            tool={(small) => (
              <button className={`ix-btn${chatOpen ? " on" : ""}${small ? " ix-btn-icon" : ""}`} data-testid="ideas-chat-toggle" aria-pressed={chatOpen} onClick={() => setChatOpen((o) => !o)}
                title="Conversations: talk this idea through with Claude, beside the editor" aria-label="Conversations">
                {dots.get(open.id) && <span className={`ix-dot ix-dot-${dots.get(open.id)}`} aria-hidden="true" />}
                {small ? <Icons.MessageSquare size={13} /> : "Conversations"}
              </button>
            )}
            buttons
            acts={[
              { testId: "ideas-relate", first: true, label: "Relate…", disabled: busy, title: "Mark another idea as related to this one", run: () => setPick({ act: "relate", id: open.id }) },
              { testId: "ideas-merge", first: true, label: "Merge into…", disabled: busy, title: "Move this idea's text into another idea; this one leaves the list", run: () => setPick({ act: "merge", id: open.id }) },
              { testId: "ideas-split", first: true, keeps: true, label: "Split…", disabled: busy || !hasSel || mode !== "edit", title: "Split the selected text into a new idea (or drag the selection onto the list; ⌥ keeps a copy)", run: askSplit },
              ...(listing?.dir ? [{ testId: "ideas-reveal", label: "Show in Finder", run: reveal }] : []),
              { testId: "ideas-remove", label: arm.armed === "remove" ? "Remove? Kept in .history" : "Remove", title: "Removes the file. A copy is kept in the folder's .history.", disabled: busy,
                armed: arm.armed === "remove", stays: arm.armed !== "remove", run: remove },
            ]}
            strips={<>
            <RelatedStrip rels={related.filter((r) => r.how !== "planned-in" && r.how !== "plan-of")} open={(id) => void openIdea(id)} />
            {open.kind === "plan" ? (
              planView && planView.doc.id === open.id && (
                <PlanPanel
                  view={planView}
                  busy={busy}
                  ideasHot={drag?.target?.kind === "plan" && drag.target.id === open.id}
                  projectsHot={drag?.target?.kind === "plan-projects"}
                  workHot={drag?.target?.kind === "work" ? drag.target.item.line : null}
                  snap={snap}
                  activity={activity}
                  gone={gonePlaces}
                  bringProject={(name) => void bringProject(open.id, name)}
                  review={setReviewing}
                  linkPlace={setPlaceFor}
                  showPlace={(root, slug) => askMain({ act: "place", root, slug })}
                  openIdea={(id) => void openIdea(id)}
                  removeIdea={(id) => void doRemoveIdea(open.id, id)}
                  addIdea={() => setAddIdeaPick(open.id)}
                  addProject={() => void openProjectPick()}
                  makeWorkspace={() => void doMakeWorkspace(open.id)}
                  openWorkspace={showWorkspace}
                  linkWorkspace={() => void openWsPick("set-plan", open.id)}
                  gotoLine={gotoLine}
                />
              )
            ) : (
              <PlanStrip
                plans={plansIn(related)}
                sources={planSources(openRow, selected, ideaRowsAll).length}
                busy={busy}
                open={(id) => void openIdea(id)}
                make={() => void doMakePlan(planSources(openRow, selected, ideaRowsAll))}
                link={() => void openWsPick("link-idea", open.id)}
              />
            )}
            </>}
          />
        ) : (
          <div className="ix-none">
            <p className="ix-none-title">{listing && listing.ideas.length === 0 ? "No ideas yet" : "No idea open"}</p>
            {up && <p className="ix-quiet">{listing && listing.ideas.length === 0 ? "Make one with New idea." : "Pick one from the list, or make a new one."}</p>}
          </div>
        )}
      </section>
      {chooser && <DropChooser x={chooser.x} y={chooser.y} from={titleOf(chooser.from)} onto={titleOf(chooser.onto)} choose={choose} />}
      {rowMenu && (
        <>
          <div className="ix-catch" onPointerDown={() => setRowMenu(null)} />
          <div className="ix-pop" role="menu" data-testid="ideas-row-menu" style={{ left: Math.min(rowMenu.x, window.innerWidth - 220), top: Math.min(rowMenu.y, window.innerHeight - 120) }}>
            <button className="ix-pop-item" role="menuitem" data-testid="ideas-menu-relate" onClick={() => { setPick({ act: "relate", id: rowMenu.idea.id! }); setRowMenu(null); }}>Relate to…</button>
            <button className="ix-pop-item" role="menuitem" data-testid="ideas-menu-merge" onClick={() => { setPick({ act: "merge", id: rowMenu.idea.id! }); setRowMenu(null); }}>Merge into…</button>
            {selected.length >= 2 && selected.includes(rowMenu.idea.id!) && (
              <button className="ix-pop-item" role="menuitem" data-testid="ideas-menu-combine" onClick={() => { setCombineAsk(selected); setRowMenu(null); }}>Combine {selected.length}…</button>
            )}
            {rowMenu.idea.kind === "plan" ? (
              <button className="ix-pop-item" role="menuitem" data-testid="ideas-menu-make-ws" onClick={() => { const id = rowMenu.idea.id!; setRowMenu(null); void doMakeWorkspace(id); }}>Make workspace</button>
            ) : (
              <>
                <button className="ix-pop-item" role="menuitem" data-testid="ideas-menu-plan" onClick={() => {
                  const id = rowMenu.idea.id!;
                  setRowMenu(null);
                  void doMakePlan(selected.includes(id) ? planSources(null, selected, ideaRowsAll) : [id]);
                }}>Make a plan{selected.length >= 2 && selected.includes(rowMenu.idea.id!) ? ` from ${planSources(null, selected, ideaRowsAll).length}` : ""}</button>
                <button className="ix-pop-item" role="menuitem" data-testid="ideas-menu-link-ws" onClick={() => { const id = rowMenu.idea.id!; setRowMenu(null); void openWsPick("link-idea", id); }}>Link to workspace…</button>
              </>
            )}
          </div>
        </>
      )}
      {pick && (
        <PickIdea
          title={pick.act === "relate" ? `Relate “${titleOf(pick.id)}” to…` : `Merge “${titleOf(pick.id)}” into…`}
          rows={ideaRows}
          exclude={pick.id}
          confirm={pick.act === "merge" ? (r) => `“${titleOf(pick.id)}” is appended to “${r.title}” and leaves the list. Its id keeps pointing at “${r.title}”; both versions are kept in History.` : undefined}
          act={(r) => { const p = pick; setPick(null); if (p.act === "relate") void doRelate(p.id, r.id!); else void doMerge(r.id!, p.id); }}
          close={() => setPick(null)}
        />
      )}
      {addIdeaPick && (
        <PickIdea
          title={`Add an idea to “${titleOf(addIdeaPick)}”`}
          rows={ideaRows.filter((r) => r.kind !== "plan" && !(planView?.doc.id === addIdeaPick && planView.ideas.some((i) => i.live === r.id)))}
          exclude={addIdeaPick}
          act={(r) => { const p = addIdeaPick; setAddIdeaPick(null); void doAddIdeas(p, [r.id!]); }}
          close={() => setAddIdeaPick(null)}
        />
      )}
      {regProjects && planView && open?.kind === "plan" && (
        <PickProject
          rows={regProjects}
          skip={planView.project_states.map((s) => s.root ?? "").filter(Boolean)}
          act={(root) => { setRegProjects(null); void doAddProject(planView.doc.id, root); }}
          arm={(e, root) => armDrag(e, { kind: "project", root })}
          close={() => setRegProjects(null)}
        />
      )}
      {reviewing && planView && open?.kind === "plan" && (() => {
        const n = planView.work?.findIndex((w) => w.line === reviewing.line && w.title === reviewing.title) ?? -1;
        const state = n < 0 ? undefined : planView.work_states[n];
        return (
          <StartReview
            key={`${open.id}|${reviewing.line}`}
            item={reviewing}
            state={state}
            at={itemPlace(n < 0 ? reviewing : planView.work![n], state, snap, activity, gonePlaces)}
            snap={snap}
            agent={agentFor(reviewing, agentPrefs)}
            start={(brief) => doStart(planView.doc.id, reviewing, state?.root ?? "", brief)}
            launch={async (root, slug) => { const c = launchCall(root, slug, agentFor(reviewing, agentPrefs)); return invoke<{ ok: boolean; output: string }>(c.cmd, c.args); }}
            show={(root, slug) => { setReviewing(null); askMain({ act: "place", root, slug }); }}
            close={() => setReviewing(null)}
          />
        );
      })()}
      {placeFor && planView && open?.kind === "plan" && (
        <PickPlace
          title={`Link a place to “${placeFor.title}” (click, or drag onto an item)`}
          snap={snap}
          root={planView.work_states.find((w) => w.line === placeFor.line)?.root}
          act={(root, slug) => { const w = placeFor; setPlaceFor(null); void doLinkPlace(planView.doc.id, w, root, slug); }}
          arm={(e, root, slug) => armDrag(e, { kind: "place", root, slug })}
          close={() => setPlaceFor(null)}
        />
      )}
      {wsPick && (
        <PickWorkspace
          title={wsPick.mode === "set-plan" ? `Make a workspace “${titleOf(wsPick.id)}”'s` : `Link “${titleOf(wsPick.id)}” to a workspace`}
          rows={wsPick.rows}
          empty={wsPick.mode === "set-plan" ? "Every workspace has a plan already. Make one with Make workspace." : "No workspace to link it to. Make one in the main window, under Workspaces."}
          act={(id) => void doWsPick(id)}
          close={() => setWsPick(null)}
        />
      )}
      {combineAsk && <CombineAsk titles={combineAsk.map(titleOf)} act={(t) => void doCombine(combineAsk, t)} close={() => setCombineAsk(null)} />}
      {splitAsk && <SplitAsk preview={splitAsk.text} act={(o) => void doSplit(splitAsk, o.extract, o.title)} close={() => setSplitAsk(null)} />}
      {open && chatOpen
        ? <IdeaChat ideaId={open.id} dir={listing?.dir ?? null} sessions={sessions} terminalCmd={prefs.terminal} gen={gen} quote={quoting ? quote : null} uiRem={prefs.uiRem} onNotice={setNotice} />
        : <aside className="ix-side" aria-hidden="true" />}
    </main>
  );
}
