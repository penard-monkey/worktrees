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
// Layout: a grid of [list | document | side]. The side column is empty and
// zero-wide in phase 1; the agent panel of phase 6 opens there, beside the
// editor, without moving either. List rows are plain elements carrying their
// id (`data-idea`), which is what phase 3's drag (navdrag.ts) will hang on.
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { EditorView } from "@codemirror/view";
import { engineState, invoke, listen, onEngineState, type EngineState } from "../src/ipc";
import { Markdown } from "../src/markdown";
import { revealOnHost } from "../src/onhost";
import { applySettings, loadSettings } from "../src/settings";
import { useArmed } from "../src/useArmed";
import "../src/tokens.css";
import "../src/App.css";
import "./companion.css";
import { ideaState, type EditorHooks } from "./editor";
import { Session, type IdeasChanged, type SaveState, type Saved } from "./ideasave";
import { savedText } from "./ideastate";
import { bodyOf, commentOnly, withId } from "./ideatext";
import { IdeaChat, ideaDots, type ConvState, type IdeaSessionsEv } from "./IdeaChat";
import { useNavDrag } from "../src/navdrag";
import { CombineAsk, DropChooser, LinkPicker, PickIdea, RelatedStrip, SplitAsk, VersionsPanel, type Relation, type Version } from "./IdeaRelate";
import {
  combineCall, combineDrop, ideaDrop, ideaHref, ideaLink, mergeCall, mergedNote, nextSelection, pickerMatches, readableLinks,
  relateCall, splitCall, versionWhen, type Call, type Choice,
} from "./ideasrelate";

// ── the rows' shapes (worktrees_core::ideas) ────────────────────────────────
type Problem = "no-id" | "bad-id" | "duplicate-id" | "too-large" | "not-text" | "unreadable";
export type Idea = { id: string | null; title: string; file: string; created: string | null; mtime_ms: number; size: number; problem?: Problem };
type Listing = { ideas: Idea[]; skipped: string[]; dir?: string };
type Merged = { id: string; title: string | null; date: string | null };
type Doc = Idea & { content: string; merged?: Merged };

const PROBLEM: Record<Problem, string> = {
  "no-id": "Made by hand. Adopt it to open it here.",
  "bad-id": "Its id is not valid. Fix or remove the id line by hand.",
  "duplicate-id": "Another file carries the same id. Remove or re-id one copy by hand.",
  "too-large": "Too large to open (over 8 MB).",
  "not-text": "Not text (UTF-8), so it cannot be opened.",
  unreadable: "Could not be read.",
};

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

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

const clock = {
  setTimeout: (fn: () => void, ms: number) => window.setTimeout(fn, ms),
  clearTimeout: (h: unknown) => window.clearTimeout(h as number),
  now: () => Date.now(),
};

function saveWords(s: SaveState): string {
  switch (s.kind) {
    case "clean":
      return s.savedAt ? `Saved ${new Date(s.savedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : "Saved";
    case "dirty":
      return "Edited";
    case "saving":
      return "Saving…";
    case "conflict":
      return "Not saved";
    case "removed":
      return "Removed";
    case "error":
      return "Not saved";
  }
}

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

// ── the editor's host ───────────────────────────────────────────────────────
export type EditorHandle = {
  text(): string; replace(content: string): void; focus(): void;
  /** The selection as offsets into `text()` (UTF-16, what a JS string
   *  counts), and its text; `null` when nothing is selected. */
  selection(): { start: number; end: number; text: string } | null;
  /** Replace document positions `from..to` with `text`, as a person's input
   *  (so it saves), and put the cursor after it. */
  insert(from: number, to: number, text: string): void;
};

/** One CodeMirror view for the window's life; a new document gets a new
 *  STATE (its own history, its own line separator). `docKey` changes when
 *  another idea is opened. */
function EditorHost(props: {
  docKey: string; initial: string; focus: boolean; handle: React.MutableRefObject<EditorHandle | null>; edited(): void; blurred(): void; hidden: boolean;
  /** Phase 3's hooks (`EditorHooks`), read through a ref so they stay current. */
  more: Pick<EditorHooks, "picker" | "pickerKey" | "follow" | "selected">;
  /** Text dragged out of the editor (an HTML5 drag CodeMirror starts). */
  textDrag(on: boolean): void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const cb = useRef({ edited: props.edited, blurred: props.blurred, more: props.more });
  cb.current = { edited: props.edited, blurred: props.blurred, more: props.more };
  const hooks = useMemo<EditorHooks>(() => ({
    edited: () => cb.current.edited(),
    blurred: () => cb.current.blurred(),
    picker: (p) => cb.current.more.picker?.(p),
    pickerKey: (k) => cb.current.more.pickerKey?.(k) ?? false,
    follow: (id) => cb.current.more.follow?.(id),
    selected: (h) => cb.current.more.selected?.(h),
  }), []);
  useEffect(() => {
    const v = new EditorView({ parent: host.current! });
    view.current = v;
    return () => { v.destroy(); view.current = null; };
  }, []);
  useEffect(() => {
    const v = view.current;
    if (!v) return;
    // The cursor starts at the end of the text: at 0 it would sit before the
    // frontmatter, where the guard refuses every keystroke.
    const end = Number.MAX_SAFE_INTEGER;
    v.setState(ideaState(props.initial, hooks, { anchor: end, head: end }));
    props.handle.current = {
      text: () => savedText(v.state),
      replace: (content) => {
        const sel = v.state.selection.main;
        v.setState(ideaState(content, hooks, { anchor: sel.anchor, head: sel.head }));
      },
      focus: () => v.focus(),
      selection: () => {
        const r = v.state.selection.main;
        if (r.empty) return null;
        return { start: v.state.sliceDoc(0, r.from).length, end: v.state.sliceDoc(0, r.to).length, text: v.state.sliceDoc(r.from, r.to) };
      },
      insert: (from, to, text) => {
        v.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length }, userEvent: "input.complete", scrollIntoView: true });
        v.focus();
      },
    };
    if (props.focus) v.focus();
    // The initial text is read once per document; a reload goes through
    // `replace`, never through a re-render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.docKey]);
  return <div ref={host} className="ix-editor" data-testid="ideas-editor" hidden={props.hidden}
    onDragStart={() => props.textDrag(true)} onDragEnd={() => props.textDrag(false)} />;
}

/** Read: the dock's markdown renderer on the body (the frontmatter is the
 *  file's, not the prose). HTML comments draw as nothing — later phases'
 *  anchors are comments (§4.4); other raw HTML stays literal, inert text. A
 *  mermaid fence is code here: mermaid lives in the docs viewer's bundle
 *  alone. Links do nothing in phase 1. */
function ReadView({ text, follow }: { text: string; follow(id: string): void }) {
  // `idea:` links become `#idea:` ones, which the shared renderer links.
  const body = useMemo(() => readableLinks(bodyOf(text)), [text]);
  return (
    <div className="ix-read" data-testid="ideas-read">
      <Markdown src={body} renderHtml={(raw) => (commentOnly(raw) ? "" : null)} onLink={(href) => { const id = ideaHref(href); if (id) follow(id); }} />
    </div>
  );
}

// ── the window ──────────────────────────────────────────────────────────────
/** The open idea. `focus`: put the cursor in the editor as it opens (a new
 *  idea), from the editor host's own effect — a timer set beside the state
 *  change can fire before React commits it (it did, in Chromium). */
type Open = { id: string; title: string; file: string; key: string; initial: string; focus: boolean };

export function Companion() {
  const engine = useEngine();
  const up = engine.kind === "up";
  const gen = engine.gen;
  const [listing, setListing] = useState<Listing | null>(null);
  const [filter, setFilter] = useState("");
  const [open, setOpen] = useState<Open | null>(null);
  const [save, setSave] = useState<SaveState>({ kind: "clean", savedAt: null });
  const [mode, setMode] = useState<"edit" | "read">("edit");
  const [readText, setReadText] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const session = useRef<Session | null>(null);
  const handle = useRef<EditorHandle | null>(null);
  const arm = useArmed<"remove" | "reload">();
  const keySeq = useRef(0);
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
  const [versions, setVersions] = useState<Version[] | null | false>(false);
  const [related, setRelated] = useState<Relation[]>([]);
  const [relTick, setRelTick] = useState(0);
  const [hasSel, setHasSel] = useState(false);
  const [picker, setPicker] = useState<{ from: number; to: number; query: string; x: number; y: number } | null>(null);
  const [pickIdx, setPickIdx] = useState(0);
  const pickerDismissed = useRef<number | null>(null);
  const textDragging = useRef(false);
  const [textDrag, setTextDrag] = useState(false);
  const listRef = useRef<HTMLUListElement | null>(null);
  const lastXY = useRef({ x: 0, y: 0 });

  // Once per socket: the window's theme from the shared settings (read only —
  // the main window owns ui-state.json's whole-blob save), and the connect
  // line. gen 0 is a door with no engine (the mock): nothing to log to.
  useEffect(() => {
    if (!up) return;
    loadSettings().then((st) => { applySettings(st); setPrefs({ terminal: st.terminal_cmd, uiRem: st.ui_rem }); }).catch(() => {});
    if (gen > 0) invoke("log_event", { level: "info", msg: connectLine(gen) }).catch(() => {});
  }, [up, gen]);

  const refresh = useCallback(async () => {
    try { setListing(await invoke<Listing>("ideas_list")); } catch (e) { setNotice(`Could not list the ideas: ${errText(e)}`); }
  }, []);
  useEffect(() => { if (up) void refresh(); }, [up, gen, refresh]);

  // An edit anywhere in the folder: re-pull the list, whatever the event
  // carries (a hand-made file has no id to name), and let the open idea's
  // session decide what it means for the editor.
  useEffect(() => {
    let off: (() => void) | null = null;
    let gone = false;
    listen<IdeasChanged>("ideas:changed", (ev) => {
      void refresh();
      setRelTick((n) => n + 1);
      const p = ev.payload;
      if (p && session.current) void session.current.external({ changed: p.changed ?? [], removed: p.removed ?? [] });
    }).then((u) => { if (gone) u(); else off = u; }).catch(() => {});
    return () => { gone = true; off?.(); };
  }, [refresh]);

  // The conversations' live state, by tmux session (serve sends the last one
  // as soon as this window declares the event).
  useEffect(() => {
    let off: (() => void) | null = null;
    let gone = false;
    listen<IdeaSessionsEv>("ideas:sessions", (ev) => { if (ev.payload) setSessions(ev.payload); })
      .then((u) => { if (gone) u(); else off = u; }).catch(() => {});
    return () => { gone = true; off?.(); };
  }, []);

  /** Start a session on a document just read, created or adopted. */
  const begin = useCallback((d: Doc, focus = false) => {
    session.current?.close();
    let known = { file: d.file, title: d.title };
    const s = new Session(d.id!, d.content, d.mtime_ms, {
      rows: {
        write: (id, content, expectedMtime, keepHistory) =>
          invoke<Saved>("idea_write", keepHistory ? { id, content, expectedMtime, keepHistory: true } : { id, content, expectedMtime }),
        read: (id) => invoke<Doc>("idea_read", { id }),
      },
      clock,
      text: () => handle.current?.text() ?? d.content,
      replace: (c) => { handle.current?.replace(c); setReadText(c); },
      onState: (st) => {
        setSave(st);
        if (st.kind === "conflict" || st.kind === "removed" || st.kind === "error") trace(gen, `idea ${d.id} ${st.kind}${st.kind === "conflict" ? ` (${st.why})` : ""}`);
      },
      // The first heading names the file: a save may have renamed it. The
      // open idea's name and title follow at once, and the list re-pulls
      // rather than wait for the poll.
      onSaved: (saved) => {
        setOpen((o) => (o && o.id === d.id ? { ...o, file: saved.file, title: saved.title } : o));
        if (saved.file !== known.file || saved.title !== known.title) {
          if (saved.file !== known.file) trace(gen, `saved ${d.id} as ${saved.file}`);
          known = saved;
          void refresh();
        }
      },
    });
    session.current = s;
    setSave({ kind: "clean", savedAt: null });
    setRenaming(null);
    arm.disarm();
    setOpen({ id: d.id!, title: d.title, file: d.file, key: `${d.id}#${++keySeq.current}`, initial: d.content, focus });
    setReadText(d.content);
    setVersions((v) => (v === false ? false : null));
    setHasSel(false);
    setPicker(null);
    setRelTick((n) => n + 1);
  }, [arm, gen, refresh]);

  /** Leave the open idea, saving first. Refused while its text cannot be
   *  saved (a conflict, or it was removed): that text exists nowhere else. */
  const leave = useCallback(async (): Promise<boolean> => {
    const s = session.current;
    if (!s) return true;
    await s.flush();
    if ((s.held() || s.current.kind === "error") && s.dirty()) {
      setNotice("This idea has text that is not saved. Keep yours or reload before you open another.");
      return false;
    }
    return true;
  }, []);

  const openIdea = useCallback(async (id: string) => {
    if (open?.id === id) return;
    if (!(await leave())) return;
    try {
      const d = await invoke<Doc>("idea_read", { id });
      // An id that was merged away lands on the idea that took it (§3).
      setNotice(d.merged ? mergedNote(d.merged, d.title) : null);
      if (d.merged) trace(gen, `opened ${id}, merged into ${d.id}`);
      if (d.id !== open?.id) begin(d);
    } catch (e) { setNotice(`Could not open it: ${errText(e)}`); void refresh(); }
  }, [open, leave, begin, refresh, gen]);

  const newIdea = useCallback(async () => {
    if (!(await leave())) return;
    try {
      const d = await invoke<Doc>("idea_create", { title: null });
      trace(gen, `created ${d.id}`);
      setMode("edit");
      begin(d, true);
      void refresh();
    } catch (e) { setNotice(`Could not make a new idea: ${errText(e)}`); }
  }, [leave, begin, refresh, gen]);

  const adopt = useCallback(async (i: Idea) => {
    if (!(await leave())) return;
    try {
      const d = await invoke<Doc>("idea_adopt", { file: i.file, expectedMtime: i.mtime_ms });
      trace(gen, `adopted ${i.file} as ${d.id}`);
      begin(d);
      void refresh();
    } catch (e) { setNotice(`Could not adopt ${i.file}: ${errText(e)}`); void refresh(); }
  }, [leave, begin, refresh, gen]);

  const rename = useCallback(async (title: string) => {
    const s = session.current;
    if (!s || !open) return;
    setRenaming(null);
    if (!title.trim() || title.trim() === open.title) return;
    await s.flush();
    if (s.held() || s.current.kind === "error") { setNotice("Save or reload this idea before renaming it."); return; }
    setBusy(true);
    try {
      const d = await invoke<Doc>("idea_rename", { id: open.id, title: title.trim(), expectedMtime: s.lastMtime });
      s.adopt(d.content, d.mtime_ms);
      setOpen((o) => (o ? { ...o, title: d.title, file: d.file } : o));
      trace(gen, `renamed ${open.id} → ${d.file}`);
      void refresh();
    } catch (e) { setNotice(`Could not rename it: ${errText(e)}`); }
    finally { setBusy(false); }
  }, [open, refresh, gen]);

  const remove = useCallback(() => arm.confirm("remove", async () => {
    const s = session.current;
    if (!s || !open) return;
    await s.flush();
    setBusy(true);
    try {
      await invoke("idea_remove", { id: open.id, expectedMtime: s.lastMtime });
      trace(gen, `removed ${open.id}`);
      s.close();
      session.current = null;
      setOpen(null);
      setNotice(`Removed “${open.title}”. Its text is kept in the folder's .history.`);
      void refresh();
    } catch (e) { setNotice(`Could not remove it: ${errText(e)}`); }
    finally { setBusy(false); }
  }), [arm, open, refresh, gen]);

  /** A removed idea's text, kept as a new idea: create, then write the text
   *  with the new id — two rows, each the ordinary one. */
  const saveAsNew = useCallback(async () => {
    if (!open) return;
    const text = handle.current?.text() ?? "";
    try {
      const d = await invoke<Doc>("idea_create", { title: open.title });
      const content = withId(text, d.id!) ?? d.content;
      const w = await invoke<Saved>("idea_write", { id: d.id, content, expectedMtime: d.mtime_ms });
      trace(gen, `kept removed ${open.id} as ${d.id}`);
      begin({ ...d, ...w, content });
      void refresh();
    } catch (e) { setNotice(`Could not save it as a new idea: ${errText(e)}`); }
  }, [open, begin, refresh, gen]);

  const closeOpen = useCallback(() => {
    session.current?.close();
    session.current = null;
    setOpen(null);
  }, []);

  // ── phase 3: the acts (§3, §6.1). Every path, drag or menu, ends in `act`
  // with one row call from ideasrelate.ts. The open idea is saved first and
  // refused while it cannot be (its text exists nowhere else); afterwards
  // the open idea is read again, through the index, so a merged-away one
  // lands on its absorber with the note.
  const act = useCallback(async <T,>(call: Call, label: string): Promise<T | null> => {
    const s = session.current;
    if (s) {
      await s.flush();
      if (s.held() || s.current.kind === "error") { setNotice("Save or reload the open idea first: its text is not saved."); return null; }
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
  }, [gen, refresh]);

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
      setNotice("Undone. Every version is still in Versions.");
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
    setNotice(`Related “${titleOf(id)}” to “${titleOf(other)}”. Undo with Versions.`);
    if (open?.id === id) await reopen(d); else setRelTick((n) => n + 1);
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
    await session.current.flush();
    const id = open.id;
    const out = await act<{ new: Doc; source: Doc }>(splitCall(id, sel.start, sel.end, extract, session.current.lastMtime, title), `${extract ? "extract" : "split"} from ${id}`);
    if (!out) return;
    setNotice(`${extract ? "Extracted" : "Split"} “${out.new.title}” into a new idea.${extract ? "" : " A link to it takes the text's place."}`);
    setUndo({ label: `Undo ${extract ? "extract" : "split"}`, run: undoBy(extract ? [] : [id], [out.new.id!]) });
    if (!extract) begin(out.source); else setRelTick((n) => n + 1);
  }, [open, act, undoBy, begin]);

  const restore = useCallback((v: Version) => arm.confirm(`restore:${v.version}` as never, async () => {
    if (!open) return;
    const s = session.current;
    if (s) {
      await s.flush();
      if (s.held() || s.current.kind === "error") { setNotice("Save or reload this idea before restoring a version."); return; }
    }
    try {
      const d = await invoke<Doc>("idea_restore", { id: open.id, version: v.version, expectedMtime: s?.lastMtime });
      trace(gen, `restored ${open.id} ${v.version}`);
      begin(d);
      setNotice(`Restored the version of ${versionWhen(v.at)}. The text it replaced is in Versions.`);
      void refresh();
    } catch (e) { setNotice(`Could not restore it: ${errText(e)}`); }
  }), [arm, open, begin, refresh, gen]);

  // The Versions list and the related ideas follow the open idea, and any
  // change in the folder.
  useEffect(() => {
    if (!open || versions === false) return;
    let gone = false;
    invoke<Version[]>("idea_versions", { id: open.id }).then((v) => { if (!gone) setVersions(v); }).catch((e) => { if (!gone) setNotice(`Could not read its versions: ${errText(e)}`); });
    return () => { gone = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open?.id, versions === false, relTick]);
  useEffect(() => {
    if (!open || !up) { setRelated([]); return; }
    let gone = false;
    invoke<Relation[]>("idea_related", { id: open.id }).then((r) => { if (!gone) setRelated(r); }).catch(() => { if (!gone) setRelated([]); });
    return () => { gone = true; };
  }, [open?.id, relTick, up]);

  // ── the drag: one idea onto another (the chooser), or the selected rows
  // onto "New idea" (Combine). navdrag's plumbing; Escape cancels there.
  type IdeaDrag = { kind: "idea"; id: string } | { kind: "ideas"; ids: string[] };
  type IdeaTarget = { kind: "onto"; id: string } | { kind: "new"; ids: string[] };
  const ideaRows = listing?.ideas ?? [];
  const { drag, arm: armDrag } = useNavDrag<IdeaTarget, IdeaDrag>({
    scrollRef: listRef,
    resolve: (item, x, y) => {
      lastXY.current = { x, y };
      const el = document.elementFromPoint(x, y) as HTMLElement | null;
      if (item.kind === "ideas") {
        const ids = el?.closest("[data-drop-new]") ? combineDrop(item.ids) : null;
        return ids ? { kind: "new", ids } : null;
      }
      const row = el?.closest<HTMLElement>("[data-idea]");
      const onto = row ? ideaDrop(item.id, ideaRows.find((i) => i.id === row.dataset.idea) ?? null) : null;
      return onto ? { kind: "onto", id: onto } : null;
    },
    commit: (item, target) => {
      if (!target) return;
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

  // ── the `[[` picker
  const pickRows = useMemo(() => (picker ? pickerMatches(ideaRows, picker.query, open?.id ?? null) : []), [picker, ideaRows, open]);
  const pickRef = useRef({ picker, pickRows, pickIdx });
  pickRef.current = { picker, pickRows, pickIdx };
  const insertLink = (r: Idea) => {
    const p = pickRef.current.picker;
    if (!p || !r.id) return;
    handle.current?.insert(p.from, p.to, ideaLink(r.title, r.id));
    trace(gen, `linked ${r.id} into ${open?.id}`);
    setPicker(null);
  };
  const editorMore: Pick<EditorHooks, "picker" | "pickerKey" | "follow" | "selected"> = {
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
    follow: (id) => void openIdea(id),
    selected: (h) => setHasSel(h),
  };
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


  // A hidden or closing window is a save point.
  useEffect(() => {
    const flush = () => { void session.current?.flush(); };
    const vis = () => { if (document.visibilityState === "hidden") flush(); };
    document.addEventListener("visibilitychange", vis);
    window.addEventListener("pagehide", flush);
    window.addEventListener("beforeunload", flush);
    return () => {
      document.removeEventListener("visibilitychange", vis);
      window.removeEventListener("pagehide", flush);
      window.removeEventListener("beforeunload", flush);
    };
  }, []);

  const switchMode = (m: "edit" | "read") => {
    if (m === "read") setReadText(handle.current?.text() ?? "");
    setMode(m);
    if (m === "edit") window.setTimeout(() => handle.current?.focus(), 0);
  };

  const words = waitingWords(engine);
  const reveal = () => {
    if (!listing?.dir) return;
    revealOnHost(open ? `${listing.dir}/${open.file}` : listing.dir).catch((e) => setNotice(`Could not show it in Finder: ${errText(e)}`));
  };

  return (
    <main className={`ix${open && chatOpen ? " ix-with-chat" : ""}`}>
      <IdeaList
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
        textDrag={textDrag}
        onTextDrop={onTextDrop}
        scrollRef={listRef}
      />
      <section className="ix-doc" data-testid="ideas-doc">
        {words && <p className="ix-engine" data-testid="companion-engine">{words}</p>}
        {notice && (
          <div className="ix-notice" data-testid="ideas-notice" role="status">
            <span>{notice}</span>
            {undo && <button className="ix-btn" data-testid="ideas-undo" onClick={() => void undo.run()}>{undo.label}</button>}
            <button className="ix-btn ix-btn-quiet" onClick={() => { setNotice(null); setUndo(null); }} aria-label="Dismiss">Dismiss</button>
          </div>
        )}
        {open ? (
          <>
            <header className="ix-head">
              {renaming !== null ? (
                <input
                  className="ix-rename"
                  data-testid="ideas-rename-input"
                  autoFocus
                  value={renaming}
                  onChange={(e) => setRenaming(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void rename(renaming);
                    if (e.key === "Escape") setRenaming(null);
                  }}
                  onBlur={() => setRenaming(null)}
                />
              ) : (
                <h1 className="ix-title" title={open.file}>{open.title || "Untitled"}</h1>
              )}
              <div className="ix-tools">
                <span className="ix-state" data-testid="ideas-save" data-state={save.kind}>{saveWords(save)}</span>
                <span className="ix-seg" role="group" aria-label="View">
                  <button className={mode === "edit" ? "on" : ""} data-testid="ideas-mode-edit" aria-pressed={mode === "edit"} onClick={() => switchMode("edit")}>Edit</button>
                  <button className={mode === "read" ? "on" : ""} data-testid="ideas-mode-read" aria-pressed={mode === "read"} onClick={() => switchMode("read")}>Read</button>
                </span>
                <button className={`ix-btn${chatOpen ? " on" : ""}`} data-testid="ideas-chat-toggle" aria-pressed={chatOpen} onClick={() => setChatOpen((o) => !o)}
                  title="Talk this idea through with Claude, beside the editor">
                  {dots.get(open.id) && <span className={`ix-dot ix-dot-${dots.get(open.id)}`} aria-hidden="true" />}Conversations
                </button>
                <button className="ix-btn" data-testid="ideas-relate" disabled={busy} onClick={() => setPick({ act: "relate", id: open.id })}
                  title="Mark another idea as related to this one">Relate…</button>
                <button className="ix-btn" data-testid="ideas-merge" disabled={busy} onClick={() => setPick({ act: "merge", id: open.id })}
                  title="Move this idea's text into another idea; this one leaves the list">Merge into…</button>
                <button className="ix-btn" data-testid="ideas-split" disabled={busy || !hasSel || mode !== "edit"} onMouseDown={(e) => e.preventDefault()} onClick={askSplit}
                  title="Split the selected text into a new idea (or drag the selection onto the list; ⌥ keeps a copy)">Split…</button>
                <button className={`ix-btn${versions !== false ? " on" : ""}`} data-testid="ideas-versions-toggle" aria-pressed={versions !== false}
                  onClick={() => setVersions((v) => (v === false ? null : false))}>Versions</button>
                <button className="ix-btn" data-testid="ideas-rename" disabled={busy} onMouseDown={(e) => e.preventDefault()} onClick={() => setRenaming(open.title)}>Rename</button>
                {listing?.dir && <button className="ix-btn" data-testid="ideas-reveal" onClick={reveal}>Show in Finder</button>}
                <button className={`ix-btn${arm.armed === "remove" ? " ix-btn-armed" : ""}`} data-testid="ideas-remove" disabled={busy} onClick={remove}
                  title="Removes the file. A copy is kept in the folder's .history.">
                  {arm.armed === "remove" ? "Remove? Kept in .history" : "Remove"}
                </button>
              </div>
            </header>
            <RelatedStrip rels={related} open={(id) => void openIdea(id)} />
            {versions !== false && <VersionsPanel versions={versions} armed={(arm.armed as string | null)?.startsWith("restore:") ? (arm.armed as string).slice(8) : null} restore={restore} close={() => setVersions(false)} />}
            {save.kind === "conflict" && (
              <div className="ix-banner" data-testid="ideas-conflict" data-why={save.why} role="alert">
                <span>{save.why === "refused"
                  ? "This idea changed on disk, so your edit was not saved. Your text is still here."
                  : "This idea was changed in another editor while you had unsaved text. Your text is still here."}</span>
                <button className="ix-btn" data-testid="ideas-keep-mine" onClick={() => void session.current?.keepMine()}
                  title="Saves your text. The other version is copied to the folder's .history first.">Keep mine (theirs goes to .history)</button>
                <button className={`ix-btn${arm.armed === "reload" ? " ix-btn-armed" : ""}`} data-testid="ideas-reload"
                  onClick={() => arm.confirm("reload", () => void session.current?.reload())}>
                  {arm.armed === "reload" ? "Drop mine and reload?" : "Reload theirs"}
                </button>
              </div>
            )}
            {save.kind === "removed" && (
              <div className="ix-banner" data-testid="ideas-removed" role="alert">
                <span>This idea was removed from the folder. Your text is still here until you close it.</span>
                <button className="ix-btn" data-testid="ideas-save-as-new" onClick={() => void saveAsNew()}>Save as a new idea</button>
                <button className="ix-btn" data-testid="ideas-close" onClick={closeOpen}>Close</button>
              </div>
            )}
            {save.kind === "error" && (
              <div className="ix-banner" data-testid="ideas-error" role="alert">
                <span>Not saved: {save.message}</span>
                <button className="ix-btn" onClick={() => void session.current?.flush()}>Try again</button>
              </div>
            )}
            <div className="ix-body">
              <EditorHost
                docKey={open.key}
                initial={open.initial}
                focus={open.focus}
                handle={handle}
                hidden={mode !== "edit"}
                edited={() => session.current?.edited()}
                blurred={() => { void session.current?.flush(); }}
                more={editorMore}
                textDrag={textDragOn}
              />
              {mode === "read" && <ReadView text={readText} follow={(id) => void openIdea(id)} />}
              {picker && mode === "edit" && <LinkPicker x={picker.x} y={picker.y} rows={pickRows} index={pickIdx} pick={insertLink} />}
            </div>
          </>
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
          </div>
        </>
      )}
      {pick && (
        <PickIdea
          title={pick.act === "relate" ? `Relate “${titleOf(pick.id)}” to…` : `Merge “${titleOf(pick.id)}” into…`}
          rows={ideaRows}
          exclude={pick.id}
          confirm={pick.act === "merge" ? (r) => `“${titleOf(pick.id)}” is appended to “${r.title}” and leaves the list. Its id keeps pointing at “${r.title}”; both versions are kept in Versions.` : undefined}
          act={(r) => { const p = pick; setPick(null); if (p.act === "relate") void doRelate(p.id, r.id!); else void doMerge(r.id!, p.id); }}
          close={() => setPick(null)}
        />
      )}
      {combineAsk && <CombineAsk titles={combineAsk.map(titleOf)} act={(t) => void doCombine(combineAsk, t)} close={() => setCombineAsk(null)} />}
      {splitAsk && <SplitAsk preview={splitAsk.text} act={(o) => void doSplit(splitAsk, o.extract, o.title)} close={() => setSplitAsk(null)} />}
      {open && chatOpen
        ? <IdeaChat ideaId={open.id} dir={listing?.dir ?? null} sessions={sessions} terminalCmd={prefs.terminal} uiRem={prefs.uiRem} onNotice={setNotice} />
        : <aside className="ix-side" aria-hidden="true" />}
    </main>
  );
}
