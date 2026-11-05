// The chat view of a place's agent, beside its terminal (docs/proposals/
// chat-session-view.md). A per-place Chat | Terminal switch in the agent cell
// picks it; the terminal stays MOUNTED underneath, hidden and inert, so the
// way back is one click with its scrollback intact (the reading overlay's
// rule). State and rules are chat.ts; this file draws them.
//
// What it is NOT: a terminal. No tool output, no diffs, no slash commands.
// Each of those says so where it would be wanted, and the terminal is named
// as the place to do it. Esc does one terminal thing here, and only that:
// while the agent works, it stops the turn, as the Stop button beside Send
// does (`agent_interrupt`, ONE Esc into the agent's pane) — only with focus
// inside the chat and no dialog, sheet or menu owning the key (useEscape).

import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type RefObject } from "react";
import {
  activityMoved, boxesOf, bytes, callFetches, callKeys, callsSay, callsWords, cardDigits, cardKey, cardTakesFocus, choiceOpen, choiceWay, clockMs, clockOf,
  dayLabel, draftIsOwnSend, draftProblem, fieldWords, foldCalls, itemsOf, keyFocusOf, lineDiff, loadPrompt, loadTurns, noSessionYet, permissionAnswerWords,
  promptBody, questionAnswerWords, selectChoice, sendAnswer, sendDraft, sendMany, setAnswerText, setBoxText, setDraft,
  splitAtRule, stopTurn, ticksOf, toggleCall, toggleTick, tookReference, turnsUnsupported, underRoot, useChat, SEND_MAX,
  type AskedStep, type CallRead, type Item, type PromptChoice, type PromptDetail, type ReadablePrompt, type Target, type ToolCall,
} from "./chat";
import { previewOf, type ChoiceWay } from "./chat";
import { referenceInDraft } from "./dnd";
// What a screen reader is told (VoiceOver): its own line, so the lanes' imports merge apart.
import { announcement, gather, hear, promptKeyOf, type Bundle, type ChatView, type Heard } from "./chat";
// Attachments (docs/proposals/chat-attachments.md): their own line, for the same reason.
import { addAttachments, attachProblem, removeAttachment, sizeWords, splitAttached, type Attachment } from "./chat";
import { loadCommands, ranItems, sendCommand, type SlashCommand } from "./chat";
import { commandProblem, commandWords, confirmStep, confirmWords, filterCommands, menuFor, slashDraft, sourceWord, type Armed } from "./slash";
import { HARNESS_LABEL, isHarness, type Harness } from "./harness";
import * as Icons from "./icons";
import { Markdown } from "./markdown";
import { useDocPathLinks } from "./useDocPathLinks";
import { BeforeCommit, useStickToBottom } from "./stickToBottom";
import { escapeDepth, useEscape } from "./useEscape";
import "./ChatPane.css";

/** While the agent works, the one visible chat re-reads its turns this often:
 *  `sessions:busy` moves only when a turn starts or ends, and a turn can run
 *  for many minutes. Nothing polls while idle, waiting, or hidden. */
const BUSY_POLL_MS = 10_000;
/** The byte count appears under the field from here. */
const COUNT_FROM = 3500;

/** The focused element, read through shadow roots (the feedback dialog is one). */
function focused(): Element | null {
  let a: Element | null = document.activeElement;
  while (a?.shadowRoot?.activeElement) a = a.shadowRoot.activeElement;
  return a;
}
/** The prompt card is on screen: laid out, not in an inert subtree, and not
 *  under the reader (⌘⇧E covers the whole space body, and is no modal). */
const onScreen = (card: HTMLElement | null) =>
  !!card && card.isConnected && card.getClientRects().length > 0 && !card.closest("[inert]") && !document.querySelector(".reading");

export type ChatPaneProps = {
  repo: string;
  slug: string;
  /** The place's path: the key of its chat state and its activity. */
  path: string;
  provider: Harness;
  /** The agent's tmux session — `agent_send`'s and `agent_turns`' target. */
  session: string;
  /** `sessions:busy` for this place and harness. */
  activity: "busy" | "waiting" | "";
  /** `sessions:done`'s last stamp for this place: a finished turn re-reads. */
  doneAt: number;
  /** `sessions:drafts`: text typed in the terminal and never sent. */
  paneDraft: { text: string; queued: boolean } | null;
  /** Settings' `ui_rem`: it moves the message box's font and not its width,
   *  so no resize tells the box to fit its text again; this does. */
  uiRem: number;
  /** The way back. */
  onTerminal: () => void;
  /** A path in a reply opens in the dock. */
  onOpenPath: (path: string, at: { line?: number; col?: number }) => void;
  /** The Files tree's reload token, so path links are re-asked on a refresh. */
  reloadToken: number;
  onError: (msg: string) => void;
};

export function ChatPane(props: ChatPaneProps) {
  const { repo, slug, path, provider, session, activity } = props;
  const target: Target = { repo, slug, path, session };
  const v = useChat(path);
  // The transcript's reader names the provider that wrote these turns. A
  // snapshot can race a provider switch; its label must not rename a Codex
  // rollout as Claude's while the new snapshot is on its way.
  const reported = isHarness(v.turns.data?.harness) ? v.turns.data.harness : null;
  const agent = HARNESS_LABEL[reported ?? provider];
  const scrollRef = useRef<HTMLDivElement>(null);
  const prompt = v.prompt.data?.waiting ? v.prompt.data : null;
  // Busy by the push, unless a stop was seen to land since it last moved.
  const busy = activity === "busy" && !v.stopped;
  const working = busy && !prompt;

  // Live: on open, and on every change of this place's state or its finish.
  const liveKey = `${session}|${activity}|${props.doneAt}`;
  useEffect(() => {
    void loadTurns(target);
    void loadPrompt(target);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the key is the trigger
  }, [liveKey]);
  useEffect(() => {
    if (activity !== "busy") return;
    const id = setInterval(() => {
      if (document.visibilityState === "visible") void loadTurns(target);
    }, BUSY_POLL_MS);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveKey]);
  const refresh = () => { void loadTurns(target); void loadPrompt(target); };
  useEffect(() => { activityMoved(path, activity === "busy"); }, [path, activity]);

  const { pathLinks, menu } = useDocPathLinks(path, null, props.reloadToken, props.onOpenPath);
  const onLink = useCallback((href: string) => {
    if (/^https?:/i.test(href)) openUrl(href).catch((e) => props.onError(`open ${href}: ${String(e)}`));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const data = v.turns.data;
  // A Codex command's output, which no rollout keeps, after the transcript's.
  const items = data ? [...itemsOf(data.turns, v.answered, v.pending, working), ...ranItems(v.ran)] : [];
  const lastReply = [...items].reverse().find((it) => it.k === "reply")?.key ?? null;
  const promptKey = promptKeyOf(prompt);
  const { toBottom, fresh, dismiss, take, reveal } = useStickToBottom(scrollRef, !!data, promptKey, lastReply);
  const said = useAnnounce(target, v, items, agent);

  // A prompt takes the keyboard from an EMPTY composer as it appears, once
  // per prompt (`cardTakesFocus`), so its keys work without a click first;
  // the composer cannot send while a prompt waits. When the card that took it
  // goes and left the keyboard on nothing, the composer gets it back. The
  // parent's effect runs after the composer's own mount focus, which is the
  // "Chat opens on a prompt" case.
  const cardRef = useRef<HTMLDivElement>(null);
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  // The announcer's key (`promptKeyOf`): the card draws a readable prompt only.
  const fp = prompt?.readable ? promptKey : null;
  const seenFp = useRef<string | null>(null);
  const tookFocus = useRef(false);
  /** When the card took it (`performance.now()`): `cardKey`'s grace. */
  const tookAt = useRef<number | null>(null);
  useEffect(() => {
    if (fp) {
      const take = cardTakesFocus({
        fingerprint: fp, seen: seenFp.current, focus: keyFocusOf(focused(), cardRef.current),
        draft: v.draft, escapeDepth: escapeDepth(), shown: onScreen(cardRef.current),
      });
      seenFp.current = fp;
      if (take && cardRef.current) {
        cardRef.current.focus({ preventScroll: true });
        tookFocus.current = true;
        tookAt.current = performance.now();
      }
    } else if (tookFocus.current) {
      tookFocus.current = false;
      tookAt.current = null;
      const a = focused();
      if ((!a || a === document.body) && escapeDepth() === 0) fieldRef.current?.focus({ preventScroll: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per prompt
  }, [fp]);

  let body: React.ReactNode;
  if (!data) {
    if (turnsUnsupported(v.turns.error)) {
      body = (
        <div className="chat-fallback" data-testid="chat-unsupported">
          <p><b>{agent}’s conversation can’t be shown here yet.</b> This app reads Claude’s and Codex’s transcripts; {agent}’s has no reader.</p>
          <p>You can still send it a line below, and answer its prompts here. Its replies are in the terminal.</p>
          <button className="ctrl sm with-icon" onClick={props.onTerminal}><Icons.SquareTerminal size={13} /> Open the terminal</button>
        </div>
      );
    } else if (v.turns.error && prompt && noSessionYet(v.turns.error)) {
      // Not a failure: Claude has no conversation until the prompt below is
      // answered (its resume picker, its folder-trust question).
      body = <p className="chat-note" data-testid="chat-no-session">{agent} has no conversation open yet.</p>;
    } else if (v.turns.error) {
      body = (
        <div className="chat-fallback" role="alert" data-testid="chat-failed">
          <p>Couldn’t read the conversation: {v.turns.error}</p>
          <button className="ctrl sm" onClick={refresh}>Try again</button>
        </div>
      );
    } else body = <p className="chat-note" role="status">Reading the conversation…</p>;
  } else if (data.reason) {
    body = <p className="chat-note" data-testid="chat-no-agent">No agent is running in this place.</p>;
  } else {
    body = (
      <>
        <p className="chat-top">
          {data.more ? `The latest ${data.turns.length} turns` : "The session’s turns"}, from its transcript.
          {" "}Type / for the agent’s commands. Tool output and diffs are in the{" "}
          <button className="chat-linkbtn" onClick={props.onTerminal}>terminal</button>.
        </p>
        {items.length === 0 && !prompt && <p className="chat-note" data-testid="chat-empty">No messages in this session yet.</p>}
        <ol className="chat-list" data-testid="chat-list">
          {items.map((it) => <ChatItem key={it.key} it={it} agent={agent} target={target} root={path} reveal={reveal} onLink={onLink} pathLinks={pathLinks} />)}
        </ol>
      </>
    );
  }

  // Esc stops a running turn, from anywhere in the chat — its field, a
  // choice, the transcript — and only then: no surface owns the key
  // (useEscape's stack), the agent is busy, no stop is already in flight.
  // It then owns the key, so App's own Escape chain does not act on it too.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== "Escape" || e.repeat || e.nativeEvent.isComposing || e.defaultPrevented) return;
    if (escapeDepth() > 0 || !busy || v.stopping) return;
    e.preventDefault();
    e.stopPropagation();
    void stopTurn(target, agent);
  };

  return (
    <div className="chat" data-testid="chat" data-drop="mention" data-drop-provider={provider} data-drop-into="chat" onKeyDown={onKeyDown}>
      <div className="sr-only" aria-live="polite" aria-atomic="true" data-testid="chat-announce"><span key={said.n}>{said.text}</span></div>
      <div className="chat-scrollwrap">
      {/* Focusable by a click (not by Tab): a click in the transcript keeps
          the keyboard in the chat, so Esc still stops a turn after it. */}
      <div className="chat-scroll" ref={scrollRef} tabIndex={-1}>
        <div className="chat-col">
          {body}
          {working && (
            <p className="chat-typing" role="status" data-testid="chat-typing">
              <span className="chat-dots" aria-hidden="true"><span /><span /><span /></span>
              {agent} is working
            </p>
          )}
          {prompt && (
            prompt.readable
              ? <PromptCard p={prompt} target={target} agent={agent} keyboard={{ card: cardRef, field: fieldRef, tookAt }} onTerminal={props.onTerminal} onLink={onLink} />
              : (
                <div className="chat-card" data-testid="chat-prompt-unreadable">
                  <div className="chat-card-head"><span className="chat-card-dot" aria-hidden="true" /><b>{agent} needs your answer in the terminal</b></div>
                  {prompt.lines?.length ? (
                    <>
                      <p>This app can’t read this prompt whole, so it can’t answer it. The bottom of {agent}’s screen:</p>
                      <pre className="chat-prompt-lines" data-testid="chat-prompt-lines">{prompt.lines.join("\n")}</pre>
                    </>
                  ) : (
                    <p>This prompt can’t be read here (taller than its pane, or one this app doesn’t know).</p>
                  )}
                  <button className="ctrl sm with-icon" onClick={props.onTerminal}><Icons.SquareTerminal size={13} /> Answer it in the terminal</button>
                </div>
              )
          )}
        </div>
      </div>
      {fresh && (
        <button className="chat-new" onClick={() => { dismiss(); toBottom(); }} data-testid="chat-new">New messages ↓</button>
      )}
      </div>
      <Composer target={target} agent={agent} fieldRef={fieldRef} waiting={prompt ? (prompt.readable ? "above" : "terminal") : null} paneDraft={props.paneDraft} followUp={v.followUp}
        busy={busy} uiRem={props.uiRem} onTerminal={props.onTerminal} onRefresh={refresh} loading={v.turns.loading || v.prompt.loading} />
      {menu}
      {/* Renders nothing: it reads the scroll before each commit (stickToBottom.ts). */}
      <BeforeCommit take={take} />
    </div>
  );
}

/** What a screen reader is told while this chat is shown (chat.ts `hear`):
 *  new replies and a prompt appearing, gathered for `ANNOUNCE_MS` and said
 *  once into the one polite region, which is keyed so the same words twice
 *  are said twice. The chat is mounted only while it is shown, so mounting is
 *  being shown; the window hiding drops what is gathered, and coming back
 *  reads the place again and counts from that read, as a place switch does. */
function useAnnounce(target: Target, v: ChatView, items: Item[], agent: string) {
  const shownAt = useRef(Date.now());
  const heard = useRef<Heard | null>(null);
  const bundle = useRef<Bundle | null>(null);
  const timer = useRef<number | undefined>(undefined);
  // Read when the timer fires or the window comes back, not when they were set.
  const latest = useRef({ target, agent });
  latest.current = { target, agent };
  const [said, setSaid] = useState({ text: "", n: 0 });
  const visible = () => document.visibilityState === "visible";
  const drop = () => { window.clearTimeout(timer.current); timer.current = undefined; bundle.current = null; };
  const flush = () => {
    timer.current = undefined;
    const b = bundle.current;
    bundle.current = null;
    const text = b && announcement(b, latest.current.agent, heard.current?.prompt?.key ?? null, visible());
    if (text) setSaid((s) => ({ text, n: s.n + 1 }));
  };
  useEffect(() => {
    const r = hear(heard.current, { items, turnsAt: v.turns.at, prompt: v.prompt.data, promptAt: v.prompt.at, answering: v.answering }, shownAt.current, visible());
    heard.current = r.heard;
    if (!r.news) return;
    const now = Date.now();
    bundle.current = gather(bundle.current, r.news, now);
    if (timer.current === undefined) timer.current = window.setTimeout(flush, bundle.current.due - now);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- a read landing, or an answer starting or ending, is the trigger
  }, [v.turns.data, v.turns.at, v.prompt.data, v.prompt.at, v.answering]);
  useEffect(() => {
    const f = () => {
      drop();
      if (!visible()) return;
      shownAt.current = Date.now();
      heard.current = null;
      void loadTurns(latest.current.target);
      void loadPrompt(latest.current.target);
    };
    document.addEventListener("visibilitychange", f);
    return () => { document.removeEventListener("visibilitychange", f); drop(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return said;
}

type Reveal = (top: HTMLElement, bottom: HTMLElement) => void;

/** A row's line is cut by the width it has now. Only the width says so, and
 *  it moves with the window, the dock and the zoom. */
function useClipped(ref: RefObject<HTMLElement | null>, watch: boolean): boolean {
  const [clipped, setClipped] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!watch || !el) { setClipped(false); return; }
    const look = () => setClipped(el.scrollWidth > el.clientWidth);
    look();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(look);
    ro.observe(el);
    return () => ro.disconnect();
    // `.chat-call-sum` is a new element when the row becomes a button.
  }, [ref, watch, ref.current]);
  return clipped;
}

/** Tells the scroll a row was opened or closed, or that what it shows landed:
 *  on a change only, never on mounting (a fold opened over an open row, or
 *  StrictMode's second run). */
function useRevealOnChange(reveal: Reveal, open: boolean, read: CallRead | undefined, top: RefObject<HTMLElement | null>, panel: RefObject<HTMLElement | null>) {
  const last = useRef({ open, read });
  useLayoutEffect(() => {
    if (last.current.open === open && last.current.read === read) return;
    last.current = { open, read };
    if (top.current) reveal(top.current, panel.current ?? top.current);
  });
}

/** What an opened call shows: plain text in a monospace block, never markup,
 *  its line breaks kept. The whole line when the row was only cut by its width;
 *  otherwise what the host answers, with the line it already had while it asks
 *  or if it no longer can, and always saying which. `title` is empty so the
 *  list's own tooltip does not sit on top of what is being read. */
function CallFull(props: { c: ToolCall; read: CallRead | undefined; id: string; panel: RefObject<HTMLDivElement | null> }) {
  const { c, read } = props;
  const fetches = callFetches(c);
  const text = fetches && read?.state === "done" ? read.text : c.summary;
  return (
    <div ref={props.panel} id={props.id} className="chat-call-full" title="" data-testid="chat-call-full">
      {text && <pre className="chat-call-text" data-testid="chat-call-text">{text}</pre>}
      {fetches && (!read || read.state === "loading") && <p className="chat-call-note" role="status">Reading the whole call…</p>}
      {read?.state === "done" && read.truncated && (
        <p className="chat-call-note" data-testid="chat-call-cut">Cut here: this is the first 64 KB of {Math.ceil(read.bytes / 1024)} KB.</p>
      )}
      {read?.state === "gone" && <p className="chat-call-note" data-testid="chat-call-gone">No longer available: this call has left the part of the transcript this app reads.</p>}
      {read?.state === "old" && <p className="chat-call-note" data-testid="chat-call-old">{read.words}.</p>}
      {read?.state === "failed" && <p className="chat-call-note" role="alert" data-testid="chat-call-failed">Couldn’t read this call: {read.words}. Close it and open it again to retry.</p>}
    </div>
  );
}

/** One call with a line: as it always was until it has more to show, and then
 *  a button that opens under itself. What is open is the store's, by `key`, so
 *  a re-read, the fold and a place switch all leave it open. Only a click does:
 *  a failed call stays closed. The text sits outside the button, so selecting
 *  it never closes it. */
function CallRow(props: { c: ToolCall; k: string; target: Target; root: string; reveal: Reveal }) {
  const { c, k, target } = props;
  const v = useChat(target.path);
  const id = useId();
  const li = useRef<HTMLLIElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const sum = useRef<HTMLSpanElement>(null);
  const fetches = callFetches(c);
  const clipped = useClipped(sum, !fetches && !!c.summary);
  const open = !!v.calls.open[k];
  const read = c.call_id ? v.calls.read[c.call_id] : undefined;
  useRevealOnChange(props.reveal, open, read, li, panel);
  const head = (
    <>
      <span className="chat-tool chat-call-name">{c.name}</span>
      {c.summary && <span ref={sum} className="chat-call-sum" title={c.summary}>{underRoot(c.summary, props.root)}</span>}
      {c.failed && <span className="chat-call-failed"><span className="chat-call-dot" aria-hidden="true" />failed</span>}
    </>
  );
  return (
    <li ref={li} className={"chat-call-row" + (c.failed ? " failed" : "")} data-testid="chat-call">
      {fetches || clipped || open ? (
        // The row is an inner span: WebKit shrink-wraps a <button> that is
        // itself the flex container (AGENTS.md, WebKit probe).
        <button type="button" className="chat-call-btn" aria-expanded={open} aria-controls={open ? id : undefined}
          onClick={() => void toggleCall(target, k, c)} data-testid="chat-call-open">
          <span className="chat-call">{head}</span>
        </button>
      ) : <div className="chat-call">{head}</div>}
      {open && <CallFull c={c} read={read} id={id} panel={panel} />}
    </li>
  );
}

/** A call with no line, as a name's pill: a button when the host can say what
 *  it was given (an MCP tool's input). What opens takes the whole row under it. */
function CallChip(props: { c: ToolCall; k: string; target: Target; reveal: Reveal }) {
  const { c, k, target } = props;
  const v = useChat(target.path);
  const id = useId();
  const li = useRef<HTMLLIElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const open = !!v.calls.open[k];
  const read = c.call_id ? v.calls.read[c.call_id] : undefined;
  useRevealOnChange(props.reveal, open, read, li, panel);
  if (!callFetches(c)) return <li className="chat-tool">{c.name}</li>;
  return (
    <>
      <li ref={li} className="chat-tool-cell">
        <button type="button" className="chat-tool chat-tool-btn" aria-expanded={open} aria-controls={open ? id : undefined}
          onClick={() => void toggleCall(target, k, c)} data-testid="chat-call-open">{c.name}</button>
      </li>
      {open && <li className="chat-tool-open"><CallFull c={c} read={read} id={id} panel={panel} /></li>}
    </>
  );
}

/** A turn's tool calls. From a host that says what each did: a row per
 *  call, its name, its line (the place's root taken off its paths, the whole
 *  line in the title) and a failed call's dot AND word, the first few shown
 *  and the rest behind "+N more", which names a failure it hides. With no
 *  line to show: the names, as chips. A call opens under itself to what it was
 *  given when there is more to see. Text only: all of it is transcript text. */
function ToolCalls(props: { calls: ToolCall[]; itemKey: string; target: Target; root: string; reveal: Reveal }) {
  const [open, setOpen] = useState(false);
  const { calls, target, reveal } = props;
  const keys = callKeys(calls, props.itemKey);
  if (!callsSay(calls)) {
    return (
      <ul className="chat-tools" title="Tools it called. Their output is in the terminal.">
        {calls.map((c, i) => <CallChip key={keys[i]} c={c} k={keys[i]} target={target} reveal={reveal} />)}
      </ul>
    );
  }
  const { shown, hidden, hiddenFailed, folds } = foldCalls(calls, open);
  return (
    <ul className="chat-calls" title="Tools it called. Their output is in the terminal." data-testid="chat-calls">
      {shown.map((c, i) => <CallRow key={keys[i]} c={c} k={keys[i]} target={target} root={props.root} reveal={reveal} />)}
      {folds && (
        <li className="chat-call-more">
          <button className="chat-linkbtn" aria-expanded={open} onClick={() => setOpen(!open)} data-testid="chat-calls-more">
            {open ? "Show fewer" : `+${hidden} more${hiddenFailed ? ` · ${hiddenFailed} failed` : ""}`}
          </button>
        </li>
      )}
    </ul>
  );
}

function ChatItem(props: { it: Item; agent: string; target: Target; root: string; reveal: Reveal; onLink: (href: string) => void; pathLinks: ReturnType<typeof useDocPathLinks>["pathLinks"] }) {
  const { it, agent } = props;
  switch (it.k) {
    case "sep":
      return <li className="chat-sep" title={new Date(it.at * 1000).toLocaleString()}>{dayLabel(it.at)}</li>;
    case "earlier":
      return <li className="chat-note">This turn began before the transcript window: its message isn’t shown.</li>;
    case "user": {
      const sent = splitAttached(it.text);
      return (
        <li className="chat-row chat-row-user">
          <div className="chat-bubble chat-bubble-user" role="group"
            aria-label={`You${it.at ? `, ${clockOf(it.at)}` : ""}`} data-testid="chat-user">
            <Markdown src={sent.text} onLink={props.onLink} />
            {sent.paths.length > 0 && <SentFiles paths={sent.paths} />}
            {it.midTurn && <p className="chat-bubble-note chat-midturn-caption">Sent while {agent} was working</p>}
          </div>
        </li>
      );
    }
    case "notice":
      return <NoticeItem it={it} />;
    case "reply":
      return (
        <li className="chat-row chat-row-agent">
          {it.label && <p className="chat-name" aria-hidden="true">{agent}{it.at ? <span className="chat-at"> · {clockOf(it.at)}</span> : null}</p>}
          <div className="chat-group" role="group"
            aria-label={`${agent}${it.at ? `, ${clockOf(it.at)}` : ""}${it.calls.length ? `. ${callsWords(it.calls)}` : ""}`}
            data-testid="chat-agent">
            {(it.text || it.truncated) && (
              <div className="chat-bubble chat-bubble-agent">
                {it.text && <Markdown src={it.text} onLink={props.onLink} pathLinks={props.pathLinks} />}
                {it.truncated && <p className="chat-bubble-note">Shortened by the host: the full reply is in the terminal.</p>}
              </div>
            )}
            {it.calls.length > 0 && <ToolCalls calls={it.calls} itemKey={it.key} target={props.target} root={props.root} reveal={props.reveal} />}
          </div>
        </li>
      );
    case "asked":
      return (
        <li className="chat-row chat-row-agent">
          {it.label && <p className="chat-name" aria-hidden="true">{agent}{it.step.at ? <span className="chat-at"> · {clockOf(it.step.at)}</span> : null}</p>}
          <AskedCard step={it.step} agent={agent} root={props.root} onLink={props.onLink} />
        </li>
      );
    case "answer":
      return <AnswerBubble step={it.step} />;
    case "answered":
      return (
        <li className="chat-note chat-answered" data-testid="chat-answered">
          {it.a.resumed !== undefined
            ? <>You resumed “{it.a.label}”{it.a.resumed ? ` (${it.a.resumed})` : ""}</>
            : <>You answered “{it.a.label}”{it.a.note ? <> with a note: “{it.a.note}”</> : null}</>}
          {" "}· {clockMs(it.a.at)}{it.a.confirmed ? "" : " · not confirmed"}
        </li>
      );
    case "pending": {
      const sent = splitAttached(it.p.text);
      return (
        <li className="chat-row chat-row-user">
          <div className="chat-bubble chat-bubble-user chat-bubble-pending" data-testid="chat-pending">
            <p>{sent.text}</p>
            {sent.paths.length > 0 && <SentFiles paths={sent.paths} />}
          </div>
          <p className="chat-status">{it.caughtUp ? "Delivered · check the terminal" : it.p.queued ? `Queued behind ${agent}’s running turn` : `Delivered · waiting for ${agent}`}</p>
        </li>
      );
    }
  }
}

/** Claude's transcript bookkeeping has its own visual place. The host gives
 *  us display text, never XML, and these rows never use the person's tint. */
function NoticeItem({ it }: { it: Extract<Item, { k: "notice" }> }) {
  const n = it.notice;
  const output = (text: string | undefined, label: string) => text ? <details className="chat-notice-details"><summary>{label}</summary><pre>{text}</pre></details> : null;
  switch (n.kind) {
    case "task":
      return <li className="chat-notice chat-notice-center" data-testid="chat-notice-task"><span className={`chat-notice-dot ${n.status === "completed" ? "done" : n.status === "failed" ? "failed" : ""}`} aria-hidden="true" />Background task {n.status}: {n.summary}</li>;
    case "peer_message":
      return <li className="chat-notice chat-notice-peer" data-testid="chat-notice-peer_message"><strong>Message from {n.from_name}</strong>{n.text.length > 220 ? <details className="chat-notice-details"><summary>Read message</summary><p>{n.text}</p></details> : <p>{n.text}</p>}</li>;
    case "command":
      return <li className="chat-notice chat-notice-compact" data-testid="chat-notice-command"><code className="chat-notice-chip">/{n.name}{n.args ? ` ${n.args}` : ""}</code>{output(n.output, "Show command output")}</li>;
    case "shell":
      return <li className="chat-notice chat-notice-compact" data-testid="chat-notice-shell"><code className="chat-notice-chip">! {n.command}</code>{output(n.output, "Show output")}{output(n.stderr, "Show errors")}</li>;
    case "interrupt":
      return <li className="chat-notice chat-notice-center" data-testid="chat-notice-interrupt">You stopped the turn</li>;
    case "summary":
      return <li className="chat-notice chat-notice-center" data-testid="chat-notice-summary"><details className="chat-notice-details"><summary>Earlier conversation summarised</summary><p>{n.text}</p></details></li>;
  }
}

/** The kind a stored file's extension says (the host chose it from the bytes). */
const kindOf = (path: string): "image" | "pdf" | "text" =>
  /\.(png|jpg|gif|webp)$/.test(path) ? "image" : path.endsWith(".pdf") ? "pdf" : "text";
const KIND_WORD = { image: "Image", pdf: "PDF", text: "Text file" } as const;

/** The files a sent message named, under its text: the host's names, which
 *  are all the transcript has. The full path is the title. */
function SentFiles(props: { paths: string[] }) {
  return (
    <ul className="chat-sent-files" aria-label={props.paths.length === 1 ? "1 attached file" : `${props.paths.length} attached files`} data-testid="chat-sent-files">
      {props.paths.map((p) => (
        <li key={p} className="chat-sent-file" title={p}>
          {kindOf(p) === "image" ? <Icons.Image size={12} className="chat-file-kind" /> : <Icons.FileText size={12} className="chat-file-kind" />}
          <span className="sr-only">{KIND_WORD[kindOf(p)]}: </span>{p.split("/").pop()}
        </li>
      ))}
    </ul>
  );
}

/** The files the next message carries: a name, a size, where its upload is,
 *  and a named ✕. Uploads run on Send, one by one. */
function AttachChips(props: { attachments: Attachment[]; sending: boolean; path: string }) {
  const n = props.attachments.length;
  return (
    <ul className="chat-chips" aria-label={n === 1 ? "1 file to send" : `${n} files to send`} data-testid="chat-chips">
      {props.attachments.map((a) => (
        <li key={a.id} className={"chat-chip" + (a.state === "failed" ? " failed" : "")} data-state={a.state} data-testid="chat-chip">
          {a.state === "uploading" && <span className="chat-spin" aria-hidden="true" />}
          <span className="chat-chip-name" title={a.converted ? `${a.name} · converted to JPEG; photo metadata, including location, removed` : a.name}>{a.name}</span>
          <span className="chat-chip-size">
            {sizeWords(a.size)}
            {a.state === "preparing" ? " · preparing" : a.state === "uploading" ? " · uploading" : a.state === "attached" ? " · uploaded" : a.state === "failed" ? " · not attached" : ""}
            {a.converted ? " · converted" : ""}
          </span>
          <button className="chat-chip-x" onClick={() => removeAttachment(props.path, a.id)} disabled={props.sending}
            aria-label={`Remove ${a.name}`} title={`Remove ${a.name}`} data-testid="chat-chip-remove">
            <Icons.X size={11} />
          </button>
        </li>
      ))}
    </ul>
  );
}

/** What the agent asked, resolved: a question with the option picked
 *  marked and described, the others muted; a tool call it asked to make; a
 *  plan, folded. Read from the transcript, whoever answered it. */
function AskedCard(props: { step: AskedStep; agent: string; root: string; onLink: (href: string) => void }) {
  const { step } = props;
  if (step.kind === "question") {
    const answers = step.answer?.answers ?? [];
    return (
      <div className="chat-asked" role="group" aria-label={`${props.agent} asked`} data-testid="chat-asked">
        {step.questions.map((q, i) => {
          const a = answers.find((x) => x.question === q.question) ?? answers[i];
          return (
            <div key={i} className="chat-asked-q">
              <p className="chat-asked-head">
                {q.header && <span className="chat-kind">{q.header}</span>}
                <span className="chat-asked-text">{q.question}</span>
              </p>
              <ul className="chat-asked-opts" aria-label={q.multi ? "Options, any number" : "Options"}>
                {q.options.map((o, j) => {
                  const on = !!a?.picked.includes(o.label);
                  return (
                    <li key={j} className={"chat-asked-opt" + (on ? " on" : "")} aria-current={on ? "true" : undefined}>
                      <span className="chat-asked-mark" aria-hidden="true">{on ? "✓" : ""}</span>
                      <span className="chat-asked-label">{o.label}</span>
                      {on && o.description && <span className="chat-asked-desc">{o.description}</span>}
                    </li>
                  );
                })}
                {a?.typed && (
                  <li className="chat-asked-opt on typed" aria-current="true">
                    <span className="chat-asked-mark" aria-hidden="true">✓</span>
                    <span className="chat-asked-label">{a.typed}</span>
                    <span className="chat-asked-desc">Typed as the answer</span>
                  </li>
                )}
              </ul>
            </div>
          );
        })}
      </div>
    );
  }
  if (step.kind === "permission") {
    return (
      <div className="chat-asked chat-asked-perm" role="group" aria-label={`${props.agent} asked to use ${step.tool}`} data-testid="chat-asked">
        <p className="chat-asked-head"><span className="chat-kind">Permission</span><span className="chat-asked-text">Asked to use <b>{step.tool}</b></span></p>
        {step.summary && <pre className="chat-asked-summary">{step.summary.startsWith(props.root + "/") ? step.summary.slice(props.root.length + 1) : step.summary}</pre>}
      </div>
    );
  }
  const title = step.plan.split("\n").find((l) => l.trim())?.replace(/^#+\s*/, "") || "The plan";
  return (
    <div className="chat-asked" role="group" aria-label={`${props.agent}’s plan`} data-testid="chat-asked">
      <p className="chat-asked-head"><span className="chat-kind">Plan</span></p>
      <details className="chat-asked-plan">
        <summary>{title}</summary>
        <div className="chat-detail-plan"><Markdown src={step.plan} onLink={props.onLink} /></div>
        {step.plan_file && <p className="chat-detail-sub">Plan file: {step.plan_file}</p>}
      </details>
    </div>
  );
}

/** The answer a person gave, on their side of the conversation: the options
 *  or text, Yes or No, approved or sent back, and any note with it. */
function AnswerBubble(props: { step: AskedStep }) {
  const { step } = props;
  const words = step.kind === "question" ? questionAnswerWords(step)
    : step.kind === "permission" ? permissionAnswerWords(step)
    : step.approved ? "Approved" : "Sent back";
  const note = step.kind === "question" ? (step.answer?.answers ?? []).map((a) => a.note).filter(Boolean).join(" · ") || undefined : step.note;
  return (
    <li className="chat-row chat-row-user">
      <div className="chat-bubble chat-bubble-user chat-bubble-answer" role="group"
        aria-label={`You answered${step.answered_at ? `, ${clockOf(step.answered_at)}` : ""}`} data-testid="chat-answer">
        <p className="chat-answer-words">{words}</p>
        {note && <p className="chat-answer-note">{note}</p>}
      </div>
      {step.answered_at && <p className="chat-status">Answered · {clockOf(step.answered_at)}</p>}
    </li>
  );
}

/** A waiting prompt, whole, with its own choices. A click selects (or
 *  ticks); "Send" sends — two acts, because an answer can run a command.
 *  The keys are the same two acts: a digit is a click on that choice, Enter
 *  is "Send" (`cardKey`). What it asks comes first, whole: `detail` when the
 *  terminal shows only part of it, then the question rows. */
function PromptCard(props: {
  p: ReadablePrompt; target: Target; agent: string;
  /** ChatPane's: the card, the composer's field, and when the card took the keyboard from it. */
  keyboard: { card: RefObject<HTMLDivElement | null>; field: RefObject<HTMLTextAreaElement | null>; tookAt: RefObject<number | null> };
  onTerminal: () => void; onLink: (href: string) => void;
}) {
  const { p, target, agent } = props;
  const v = useChat(target.path);
  const ids = useId();
  const rows = promptBody(p);
  const { above, question } = p.detail ? splitAtRule(rows) : { above: [] as string[], question: rows };
  const { boxes, freeBox } = p.multi ? boxesOf(p) : { boxes: [] as PromptChoice[], freeBox: null };
  const others = p.multi ? p.choices.filter((c) => c.checked === undefined) : p.choices;
  const ticks = p.multi ? ticksOf(v, p) : [];
  const selected = others.find((c) => c.n === v.selected) ?? null;
  const way = selected ? choiceWay(p, selected) : null;
  const typed = way && way.how !== "plain" ? v.answerText : "";
  const typedProblem = way && way.how !== "plain" ? draftProblem(typed, way.how === "field" ? "answer" : "note") : null;
  const canAnswer = !!selected && !!way?.open && !typedProblem && !(way.how === "field" && !typed.trim()) && !v.answering;
  const freeOn = !!freeBox && ticks.includes(freeBox.n);
  const boxProblem = freeOn ? (v.boxText.trim() ? draftProblem(v.boxText, "answer") : "Type your own answer in its box, or untick it.") : null;
  const canSendMany = !!p.multi && !selected && ticks.length > 0 && !boxProblem && !v.answering;
  // Claude's resume picker: its rows ARE the choices (the screen above them
  // is its search box and key hints), and only those on screen are offered.
  const resume = p.picker === "resume";
  // A list a slash command opened: a model or effort list (answered for this
  // session only), a dialog (its one choice, Close), or another list.
  const listed = p.picker === "model" || p.picker === "effort";
  const kind = resume ? "Resume" : listed ? "Model" : p.picker === "view" ? "Screen" : p.picker === "menu" ? "List"
    : p.detail?.tool === "ExitPlanMode" ? "Plan" : p.kind === "question" ? "Question" : "Permission";
  const heading = resume ? `${agent} is asking which conversation to resume`
    : p.picker === "model" ? `${agent}’s models` : p.picker === "effort" ? `${agent}’s reasoning effort`
    : p.picker === "view" ? `${agent} is showing a screen` : `${agent} needs your answer`;
  const answer = () => { if (canAnswer) void sendAnswer(target, agent); };
  const many = () => { if (canSendMany) void sendMany(target); };
  // Enter's send: what the Send button on screen runs, behind its own enabled state.
  const send = selected && way ? { on: canAnswer, run: answer } : p.multi ? { on: canSendMany, run: many } : null;
  const digits = cardDigits(p);
  const quote = (s: string) => `“${s.length > 40 ? s.slice(0, 39) + "…" : s}”`;
  // Claude's question with previews: the selected option's beside the
  // choices on a wide card, under it on a narrow one (`.chat-pv-wrap`).
  const pv = previewOf(p, v.selected);

  // The keys. ONE capture-phase listener on `window`, the card's while it is
  // mounted (it is mounted only while a readable prompt is shown). Not the
  // card's own `onKeyDown`: the keyboard is often NOT inside the card — on
  // the body after a click on the conversation, on the Chat switch or a rail
  // button Chromium focused on its click — and a container's handler would
  // hear nothing. Capture, so the key is decided before anything under the
  // focus acts on it: Chromium focuses a clicked choice, and a native Enter
  // there is a SECOND click (unselecting it), while WebKit focuses the card
  // around it (its nearest focusable), where Enter does nothing; held here,
  // Enter sends, once, in both. Not `useEscape`: that is Escape for modal
  // surfaces, and this card is not one — it steps aside for every one of
  // them (`escapeDepth`). The dock shell keeps its digits: a terminal, like
  // any text field, is never the card's (`keyFocusOf`). A key the card does
  // not take is left alone, undefaulted, for whoever comes next (App's ⌘1–9
  // needs ⌘, which this never takes).
  const [keyPick, setKeyPick] = useState<{ n: number; t: number } | null>(null);
  const live = useRef({ p, path: target.path, selected: v.selected, answering: v.answering, send });
  live.current = { p, path: target.path, selected: v.selected, answering: v.answering, send };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { card: cardEl, field, tookAt } = props.keyboard;
      const card = cardEl.current;
      const L = live.current;
      const k = cardKey(e, {
        focus: keyFocusOf(focused(), card), sinceTook: tookAt.current === null ? null : performance.now() - tookAt.current,
        escapeDepth: escapeDepth(), shown: onScreen(card), answering: L.answering, selected: L.selected, canSend: !!L.send?.on,
      }, L.p);
      if (!k) return;
      // Typing a message: moved now, inside keydown, the key's text lands in
      // the composer, so nothing is prevented.
      if (k.act === "composer") { field.current?.focus({ preventScroll: true }); return; }
      e.preventDefault();
      e.stopPropagation();
      if (k.act === "send") L.send?.run();
      else if (k.act === "tick") toggleTick(L.path, k.n);
      else if (k.act === "select") {
        selectChoice(L.path, k.choice);
        // ChoiceRow puts the keyboard in the text the choice takes.
        if (k.field) setKeyPick((cur) => ({ n: k.n, t: (cur?.t ?? 0) + 1 }));
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reads `live`
  }, []);
  return (
    <div ref={props.keyboard.card} className="chat-card chat-prompt" role="group" aria-labelledby={`${ids}-h`} tabIndex={-1} data-testid="chat-prompt"
      // A pointer on the card is a person who has seen it: its keys are theirs at once.
      onPointerDown={() => { props.keyboard.tookAt.current = null; }}>
      <div className="chat-card-head">
        <span className="chat-card-dot" aria-hidden="true" />
        <b id={`${ids}-h`}>{heading}</b>
        <span className="chat-kind">{kind}</span>
      </div>
      {p.detail && <PromptDetail d={p.detail} root={target.path} agent={agent} onLink={props.onLink} />}
      {above.length > 0 && (
        <details className="chat-screen" data-testid="chat-prompt-screen">
          <summary>The part the terminal shows ({above.length} rows)</summary>
          <pre className="chat-prompt-lines">{above.join("\n")}</pre>
        </details>
      )}
      {!resume && <pre className="chat-prompt-lines" data-testid="chat-prompt-lines">{question.join("\n")}</pre>}
      {listed && (
        <p className="chat-hint" data-testid="chat-session-only">
          The chat chooses for this session only. A default for new sessions is set in the terminal.
        </p>
      )}
      {resume && !!p.hidden && (
        <p className="chat-hint" data-testid="chat-resume-hidden">
          {p.choices.length} of {p.choices.length + p.hidden} shown. {p.hidden === 1 ? "The other one is" : `The other ${p.hidden} are`} in the terminal’s list.
          {" "}<button className="chat-linkbtn" onClick={props.onTerminal}>Open the terminal</button>
        </p>
      )}
      {boxes.length > 0 && (
        <div className="chat-boxes" role="group" aria-label="Tick any, then send them" data-testid="chat-boxes">
          {boxes.map((c) => {
            const on = ticks.includes(c.n);
            const { open, note } = choiceOpen(c);
            const id = `${ids}-b${c.n}`;
            if (c === freeBox) {
              return (
                <div key={c.n} className={"chat-box chat-box-free" + (on ? " on" : "")}>
                  <span className="chat-choice-n" aria-hidden="true">{c.n}</span>
                  <input id={id} type="checkbox" checked={on} disabled={v.answering}
                    aria-keyshortcuts={c.n <= 9 ? String(c.n) : undefined}
                    onChange={() => toggleTick(target.path, c.n)} aria-label={`${c.label}: your own answer`} data-testid={`chat-box-${c.n}`} />
                  <input type="text" className="chat-answer-field" value={v.boxText} placeholder="Type your own answer"
                    aria-label="Your own answer" aria-invalid={on && boxProblem ? true : undefined} disabled={v.answering}
                    autoCorrect="off" autoCapitalize="off" autoComplete="off" spellCheck={false}
                    onChange={(e) => setBoxText(target.path, e.currentTarget.value)}
                    onKeyDown={(e) => { if (e.key === "Enter" && !e.nativeEvent.isComposing) { e.preventDefault(); many(); } }}
                    data-testid="chat-box-text" />
                </div>
              );
            }
            return (
              <label key={c.n} htmlFor={id} className={"chat-box" + (on ? " on" : "") + (open ? "" : " closed")}>
                <span className="chat-choice-n" aria-hidden="true">{c.n}</span>
                <input id={id} type="checkbox" checked={on} disabled={!open || v.answering}
                  aria-keyshortcuts={c.n <= 9 ? String(c.n) : undefined}
                  onChange={() => toggleTick(target.path, c.n)} data-testid={`chat-box-${c.n}`} />
                <span className="chat-choice-text">
                  <span className="chat-choice-label">{c.label}</span>
                  {c.more?.map((m, i) => <span key={i} className="chat-choice-more">{m}</span>)}
                  {note && <span className="chat-choice-note">{note}</span>}
                </span>
              </label>
            );
          })}
        </div>
      )}
      {others.length > 0 && (
        <div className={p.previews ? "chat-pv-wrap" : undefined}>
          <div className={p.previews ? "chat-pv" : undefined} data-testid={p.previews ? "chat-preview-grid" : undefined}>
            <div className="chat-choices" role="group" aria-label={p.multi ? "Or instead" : "Choices"}>
              {others.map((c) => (
                <ChoiceRow key={c.n} c={c} way={choiceWay(p, c)} on={v.selected === c.n} busy={v.answering} path={target.path}
                  harness={p.harness} agent={agent} text={v.answerText} problem={v.selected === c.n ? typedProblem : null} onEnter={answer}
                  focusToken={keyPick?.n === c.n ? keyPick.t : 0}
                  under={pv?.c.n === c.n ? <ChoicePreview c={c} known={pv.known} where="inline" /> : null} />
              ))}
            </div>
            {pv && <ChoicePreview c={pv.c} known={pv.known} where="side" />}
          </div>
        </div>
      )}
      {p.notes?.text && (
        <p className="chat-hint chat-notes" data-testid="chat-notes-held">
          A note typed in the terminal goes with whichever answer is sent: {quote(p.notes.text)}
        </p>
      )}
      {p.notes?.open && !p.notes.text && (
        <p className="chat-hint chat-notes" data-testid="chat-notes-open">Someone is typing a note into this question in the terminal.</p>
      )}
      {v.answerWords && (
        <p className="chat-alert" role="alert" data-testid="chat-answer-line">
          {v.answerWords.line}
          {v.answerWords.terminal && <> <button className="chat-linkbtn" onClick={props.onTerminal}>Open the terminal</button></>}
        </p>
      )}
      {boxProblem && !selected && <p className="chat-hint chat-box-hint">{boxProblem}</p>}
      <div className="chat-card-foot">
        {selected && way ? (
          <>
            <button className="enter-btn sm" disabled={!canAnswer} onClick={answer} aria-keyshortcuts="Enter" data-testid="chat-send-answer">
              {v.answering ? (resume ? "Resuming…" : "Answering…")
                : way.how === "field" ? (typed.trim() ? `Send answer: ${quote(typed)}` : "Type an answer to send")
                : way.how === "amend" && typed.trim() ? `Send ${quote(selected.label)} with the note`
                : resume ? `Resume ${quote(selected.label)}`
                : `Send answer: ${quote(selected.label)}`}
            </button>
            <button className="ctrl sm" disabled={v.answering} onClick={() => selectChoice(target.path, null)}>Cancel</button>
          </>
        ) : p.multi ? (
          <>
            <button className="enter-btn sm" disabled={!canSendMany} onClick={many} aria-keyshortcuts="Enter" data-testid="chat-send-many">
              {v.answering ? "Sending…" : ticks.length ? `Send ${ticks.length} ${ticks.length === 1 ? "choice" : "choices"}` : "Tick one or more to send"}
            </button>
          </>
        ) : (
          <span className="chat-hint">{resume ? "Pick a conversation, then resume it." : "Pick a choice, then send it."}</span>
        )}
        {digits && !v.answering && <span className="chat-hint" data-testid="chat-keys-hint">Press {digits}, then Enter.</span>}
        <button className="chat-linkbtn chat-foot-end" onClick={props.onTerminal}>Answer in the terminal</button>
      </div>
    </div>
  );
}

/** What Claude shows beside an option of a `previews` question: its preview,
 *  monospaced with its spacing kept, never wrapped (a mockup's columns would
 *  move). `known: "shown"` is the terminal's one preview, all this host can
 *  see without pressing a key. */
function ChoicePreview(props: { c: PromptChoice; known: "all" | "shown"; where: "side" | "inline" }) {
  const { c } = props;
  const rows = c.preview_hidden ?? 0;
  return (
    <figure className={`chat-pv-box chat-pv-${props.where}`} data-testid={`chat-preview-${props.where}`}>
      <figcaption className="chat-pv-cap">
        {c.preview === undefined ? <>“<span className="chat-pv-label">{c.label}</span>” has no preview</> : <>Preview of “<span className="chat-pv-label">{c.label}</span>”</>}
      </figcaption>
      {c.preview !== undefined && <pre className="chat-pv-text" tabIndex={0}>{c.preview}</pre>}
      {rows > 0 && <p className="chat-pv-more">{rows === 1 ? "1 more row is" : `${rows} more rows are`} cut off in the terminal.</p>}
      {props.known === "shown" && <p className="chat-pv-more">The terminal shows one preview at a time: this is the one it shows now.</p>}
    </figure>
  );
}

/** One choice of a prompt: a button that selects it and, once selected, the
 *  text it takes — a field's answer, required, or an optional note. `way` is
 *  `choiceWay`'s, in its prompt. */
function ChoiceRow(props: {
  c: PromptChoice; way: ChoiceWay; on: boolean; busy: boolean; path: string; harness: string; agent: string;
  /** Shown right under the choice, above its text: its preview on a narrow card. */
  under?: React.ReactNode;
  text: string; problem: string | null; onEnter: () => void;
  /** Moves when a DIGIT selects this choice: its text takes the keyboard,
   *  a note's too, since the next keys are typed there (Enter sends). */
  focusToken: number;
}) {
  const { c, on } = props;
  const { open, how, note } = props.way;
  const field = useRef<HTMLInputElement>(null);
  const ids = useId();
  // A field IS the answer: selecting it is starting to type. A note is
  // optional, and a click leaves focus where it was.
  useEffect(() => { if (on && how === "field") field.current?.focus({ preventScroll: true }); }, [on, how]);
  const usedToken = useRef(0);
  useEffect(() => {
    if (!on || how === "plain" || props.focusToken <= usedToken.current) return;
    usedToken.current = props.focusToken;
    field.current?.focus({ preventScroll: true });
  }, [on, how, props.focusToken]);
  return (
    <>
      <button className={"chat-choice" + (on ? " on" : "") + (open ? "" : " closed")}
        aria-pressed={open ? on : undefined} disabled={!open || props.busy} aria-keyshortcuts={c.n <= 9 ? String(c.n) : undefined}
        onClick={() => selectChoice(props.path, on ? null : c.n)} data-testid={`chat-choice-${c.n}`}>
        {/* The grid is an inner span: WebKit shrink-wraps a <button>
            that is itself the container (AGENTS.md, WebKit probe). */}
        <span className="chat-choice-in">
          <span className="chat-choice-n" aria-hidden="true">{c.n}</span>
          <span className="chat-choice-text">
            <span className="chat-choice-label">{c.label}</span>
            {c.more?.map((m, i) => <span key={i} className="chat-choice-more">{m}</span>)}
            {note && <span className="chat-choice-note">{note}</span>}
            {!on && how === "amend" && <span className="chat-choice-note">Takes a note</span>}
          </span>
        </span>
      </button>
      {props.under}
      {on && how !== "plain" && (
        <div className="chat-answer">
          <label htmlFor={`${ids}-t`} className="sr-only">{how === "field" ? `Your answer for “${c.label}”` : `A note with “${c.label}”`}</label>
          <input id={`${ids}-t`} ref={field} type="text" className="chat-answer-field" value={props.text}
            placeholder={fieldWords(c, props.harness, props.agent)} disabled={props.busy}
            aria-invalid={props.problem ? true : undefined} aria-describedby={props.problem ? `${ids}-p` : undefined}
            autoCorrect="off" autoCapitalize="off" autoComplete="off" spellCheck={false}
            onChange={(e) => setAnswerText(props.path, e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.nativeEvent.isComposing) { e.preventDefault(); props.onEnter(); } }}
            data-testid={`chat-answer-text-${c.n}`} />
          {props.problem && <p id={`${ids}-p`} className="chat-line alert">{props.problem}</p>}
        </div>
      )}
    </>
  );
}

/** What a prompt taller than the pane asks, whole, per tool: an edit as a
 *  diff, a file as its text, a command as itself, a plan as markdown, and
 *  anything else as its JSON. Data the agent wrote, never instructions. */
function PromptDetail(props: { d: PromptDetail; root: string; agent: string; onLink: (href: string) => void }) {
  const { d, root } = props;
  const input = d.input ?? {};
  const str = (k: string) => (typeof input[k] === "string" ? (input[k] as string) : null);
  const rel = (path: string | null) => (path && path.startsWith(root + "/") ? path.slice(root.length + 1) : path);
  const file = rel(str("file_path"));
  let head: string;
  let body: React.ReactNode;
  const edits = Array.isArray(input.edits) ? (input.edits as Record<string, unknown>[]) : null;
  if (d.tool === "Edit" && file && str("old_string") !== null && str("new_string") !== null) {
    head = `Edit ${file}${input.replace_all === true ? " (every occurrence)" : ""}`;
    body = <DiffRows a={str("old_string")!} b={str("new_string")!} />;
  } else if (d.tool === "MultiEdit" && file && edits?.every((e) => typeof e.old_string === "string" && typeof e.new_string === "string")) {
    head = `Edit ${file}: ${edits.length} ${edits.length === 1 ? "change" : "changes"}`;
    body = edits.map((e, i) => (
      <div key={i} className="chat-detail-part">
        <p className="chat-detail-sub">Change {i + 1} of {edits.length}{e.replace_all === true ? " (every occurrence)" : ""}</p>
        <DiffRows a={e.old_string as string} b={e.new_string as string} />
      </div>
    ));
  } else if (d.tool === "Write" && file && str("content") !== null) {
    const n = str("content")!.split("\n").length;
    head = `Write ${file} (${n} ${n === 1 ? "line" : "lines"})`;
    body = <pre className="chat-detail-code">{str("content")}</pre>;
  } else if (d.tool === "Bash" && str("command") !== null) {
    head = "Run a command";
    body = (
      <>
        {str("description") && <p className="chat-detail-sub">{str("description")}</p>}
        <pre className="chat-detail-code">{str("command")}</pre>
      </>
    );
  } else if (d.tool === "ExitPlanMode" && str("plan") !== null) {
    head = `${props.agent}’s plan`;
    body = (
      <>
        <div className="chat-detail-plan"><Markdown src={str("plan")!} onLink={props.onLink} /></div>
        {str("planFilePath") && <p className="chat-detail-sub">Plan file: {str("planFilePath")}</p>}
      </>
    );
  } else {
    head = `${d.tool}: its input`;
    body = <pre className="chat-detail-code">{JSON.stringify(input, null, 2)}</pre>;
  }
  return (
    <section className="chat-detail" aria-label={head} data-testid="chat-detail">
      <p className="chat-detail-head">
        <span className="chat-detail-title">{head}</span>
        <span className="chat-detail-from">{d.from === "plan-file" ? "In full, from its plan file" : `In full, as ${props.agent} asked`}</span>
      </p>
      {body}
    </section>
  );
}

function DiffRows(props: { a: string; b: string }) {
  const rows = lineDiff(props.a, props.b);
  return (
    <div className="chat-diff" role="group" aria-label={`${rows.filter((r) => r.k === "-").length} lines out, ${rows.filter((r) => r.k === "+").length} in`}>
      {rows.map((r, i) => (
        <div key={i} className={"chat-diff-row" + (r.k === "-" ? " del" : r.k === "+" ? " add" : "")}>
          <span className="chat-diff-k" aria-hidden="true">{r.k}</span>{r.text || " "}
        </div>
      ))}
    </div>
  );
}

function Composer(props: {
  /** Where a waiting prompt is answered: on the card above, or in the terminal. */
  target: Target; agent: string; waiting: "above" | "terminal" | null; paneDraft: { text: string; queued: boolean } | null;
  /** The field, ChatPane's: a prompt card takes the keyboard from it and gives it back. */
  fieldRef: RefObject<HTMLTextAreaElement | null>;
  /** What to type next, after an answer that hands over to the composer. */
  followUp: string | null;
  /** The agent is mid-turn (`sessions:busy`): Stop is offered. */
  busy: boolean;
  /** Settings' `ui_rem` (ChatPane's): a fit trigger that moves with the font. */
  uiRem: number;
  onTerminal: () => void; onRefresh: () => void; loading: boolean;
}) {
  const { target, agent } = props;
  const v = useChat(target.path);
  const field = props.fieldRef;
  const ids = useId();
  const text = v.draft;
  const files = v.attachments.length;
  // A line that starts with `/` is a slash command: the session's own list
  // (`agent_commands`) decides what it may be, and `agent_command` runs it.
  const slash = slashDraft(text);
  const cmds = v.commands.data?.commands ?? null;
  const problem = slash
    ? (files ? "Files go with a message, not a command: remove them, or write a message." : commandProblem(cmds, text, (a) => draftProblem(a)) ?? (v.commands.error && !cmds ? commandWords(v.commands.error, agent).line : null))
    : files ? attachProblem(text, files) : draftProblem(text);
  const cmd: SlashCommand | null = slash && cmds ? cmds.find((c) => c.name === slash.name) ?? null : null;
  useEffect(() => { if (slash) void loadCommands(target); }, [!!slash, target.session]); // eslint-disable-line react-hooks/exhaustive-deps
  // The menu: while a name is typed, until Esc closes it for this draft.
  const [closedAt, setClosedAt] = useState<string | null>(null);
  // Closed for THAT draft: any change to it opens the menu again.
  useEffect(() => { if (closedAt !== null && text !== closedAt) setClosedAt(null); }, [text, closedAt]);
  const query = menuFor(text);
  const menuOn = query !== null && closedAt !== text && !props.waiting;
  const rows = menuOn && cmds ? filterCommands(cmds, query) : [];
  const [hi, setHi] = useState(0);
  useEffect(() => setHi(0), [query]);
  useEscape(() => setClosedAt(text), menuOn);
  // A confirm's first press, and when it lapses.
  const [armed, setArmed] = useState<Armed | null>(null);
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(null), Math.max(0, armed.until - Date.now()) + 10);
    return () => clearTimeout(t);
  }, [armed]);
  useEffect(() => setArmed(null), [text]);
  const n = bytes(text);
  const picker = useRef<HTMLInputElement>(null);
  const [dropping, setDropping] = useState(false);
  const withFiles = (e: React.DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes("Files");

  // Switching to Chat is switching to typing here: the hidden terminal has
  // just given the keyboard up (App's `setChatView`). Never over a prompt card
  // that has taken it (ChatPane): StrictMode runs this twice.
  useEffect(() => {
    if (!document.activeElement?.closest(".chat-prompt")) field.current?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => { if (props.followUp) field.current?.focus({ preventScroll: true }); }, [props.followUp]);

  // Grow with the text, to six lines; a long line wraps, it never scrolls
  // sideways. The field scrolls only past the cap: with "always show scroll
  // bars" a sub-pixel shortfall (a 1.45 line height against an integer
  // scrollHeight) drew a vertical bar, whose width then drew a horizontal one.
  const fit = useCallback(() => {
    const el = field.current;
    // Hidden (display: none): nothing to measure; the observer refits it on show.
    if (!el || el.clientWidth === 0) return;
    // Measured with no scroll bar, so the over-the-cap call is made at the
    // full width whatever the last fit decided (a classic bar narrows it).
    el.style.overflowY = "hidden";
    el.style.height = "auto";
    const cs = getComputedStyle(el);
    const line = parseFloat(cs.lineHeight) || 20;
    const pad = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
    const border = parseFloat(cs.borderTopWidth) + parseFloat(cs.borderBottomWidth);
    const cap = Math.ceil(line * 6 + pad);
    const over = el.scrollHeight > cap;
    el.style.overflowY = over ? "auto" : "hidden";
    el.style.height = `${Math.ceil(Math.min(el.scrollHeight, cap) + border)}px`;
  }, [field]);
  // The UI size moves the font, not the width (App writes `--ui-rem` before
  // this render), so it is a trigger of its own.
  useLayoutEffect(fit, [text, props.uiRem, fit]);
  // A width change rewraps the text without touching it: the window, the dock,
  // the 560px gutter switch. Refit then, and only then (a height change is ours).
  useEffect(() => {
    const el = field.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    let w = -1;
    const ro = new ResizeObserver((es) => {
      const cw = es[es.length - 1].contentRect.width;
      if (cw === w) return;
      w = cw;
      fit();
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [field, fit]);

  // A place dropped on the chat (App's `commitDrop`): only here is the cursor
  // known. By id, because StrictMode runs this twice on a mount that finds one.
  const tookInsert = useRef(0);
  const caret = useRef<number | null>(null);
  useLayoutEffect(() => {
    const el = field.current;
    if (!v.insert || !el || tookInsert.current === v.insert.id) return;
    tookInsert.current = v.insert.id;
    const next = referenceInDraft(text, el.selectionStart, el.selectionEnd, v.insert.text);
    caret.current = next.caret;
    tookReference(target.path, next.draft);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the insert is the trigger
  }, [v.insert]);
  useLayoutEffect(() => {
    const el = field.current;
    if (!el || caret.current === null) return;
    el.focus({ preventScroll: true });
    el.setSelectionRange(caret.current, caret.current);
    caret.current = null;
  }, [text, field]);

  const preparing = v.attachments.some((a) => a.state === "preparing");
  const canSend = (!!text.trim() || files > 0) && !problem && !props.waiting && !v.sending && !preparing;
  const runCommand = () => {
    if (!slash || !cmd) return;
    const args = (slash.args ?? "").trim();
    const step = confirmStep(cmd, `/${cmd.name}${args ? " " + args : ""}`, armed, Date.now());
    setArmed(step.armed);
    if (step.run) void sendCommand(target, cmd.name, args, (e) => commandWords(e, agent));
  };
  const send = () => { if (!canSend) return; if (slash) runCommand(); else void sendDraft(target, agent); };
  // A menu row: the exact name runs (Enter), any other fills the field.
  const pick = (c: SlashCommand, how: "enter" | "tab" | "click") => {
    if (how !== "tab" && c.name === query) { setClosedAt(text); send(); return; }
    setDraft(target.path, `/${c.name}${how === "tab" && c.args ? " " : ""}`);
    field.current?.focus({ preventScroll: true });
  };
  // Only a line break or a tab is wrong: offer to join, by hand, never silently.
  const joinable = !!problem && /[\n\r\t]/.test(text) && !draftProblem(text.replace(/\s*[\r\n\t]+\s*/g, " ").trim());

  const stop = () => { if (!v.stopping) void stopTurn(target, agent); };
  const said = v.sendWords ?? v.attachWords ?? v.stopWords;

  let line: React.ReactNode;
  let alert = false;
  const up = v.attachments.findIndex((a) => a.state === "uploading");
  if (preparing) line = "Preparing the file…";
  else if (v.sending && up >= 0) line = files === 1 ? "Uploading the file…" : `Uploading ${up + 1} of ${files}…`;
  else if (v.sending) line = "Sending…";
  else if (v.stopping) line = `Stopping ${agent}’s turn…`;
  else if (v.sendWords) { line = v.sendWords.line; alert = true; }
  else if (v.attachWords) { line = v.attachWords.line; alert = true; }
  // A stop's words: a refusal or an unconfirmed stop is an alert; "Stopped." is not.
  else if (v.stopWords) { line = v.stopWords.line; alert = !!(v.stopWords.terminal || v.stopWords.prompt); }
  else if (armed && cmd) { line = `Press Enter again to run /${cmd.name}. ${confirmWords(cmd) ?? ""}`; alert = true; }
  // Still choosing from the menu: how to pick, not what is wrong.
  else if (menuOn && rows.length && !cmd) line = "↑↓ to choose · Enter or Tab to pick · Esc to close";
  else if (problem) { line = problem; alert = true; }
  else if (props.waiting) line = `Messages can’t be sent while a prompt is waiting. Answer it ${props.waiting === "above" ? "above" : "in the terminal"}.`;
  else if (props.followUp && !text.trim()) line = props.followUp;
  else if (cmd) line = `Enter runs /${cmd.name}${cmd.tier === "sensitive" ? " · a sensitive command, confirmed first" : confirmWords(cmd) ? " · confirmed first" : ""}`;
  else line = n >= COUNT_FROM ? `One line · ${n.toLocaleString()} of ${SEND_MAX.toLocaleString()} bytes` : "One line · Enter sends · / for commands";

  return (
    <div className={"chat-composer" + (dropping ? " dropping" : "")} data-testid="chat-composer"
      onDragOver={(e) => {
        if (!withFiles(e) || v.attachOff) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
        setDropping(true);
      }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false); }}
      onDrop={(e) => {
        setDropping(false);
        if (!withFiles(e) || v.attachOff) return;
        e.preventDefault();
        addAttachments(target.path, Array.from(e.dataTransfer.files));
        field.current?.focus({ preventScroll: true });
      }}>
      {props.paneDraft && !props.waiting && !draftIsOwnSend(props.paneDraft.text, v) && (
        <p className="chat-paneDraft" data-testid="chat-pane-draft">
          The terminal’s input holds unsent text (“{props.paneDraft.text.length > 60 ? props.paneDraft.text.slice(0, 59) + "…" : props.paneDraft.text}”).
          {" "}{agent} takes no message until it is sent or cleared{" "}
          <button className="chat-linkbtn" onClick={props.onTerminal}>in the terminal</button>.
        </p>
      )}
      {files > 0 && <AttachChips attachments={v.attachments} sending={v.sending} path={target.path} />}
      {menuOn && (
        <SlashMenu id={`${ids}-m`} rows={rows} hi={hi} loading={!cmds && v.commands.loading}
          error={!cmds && v.commands.error ? commandWords(v.commands.error, agent).line : null}
          onHover={setHi} onPick={(c) => pick(c, "click")} />
      )}
      <div className="chat-composer-row">
        {/* A plain file input: it reads bytes, never a path, so it works the
            same through the remote door. */}
        <input ref={picker} type="file" multiple hidden tabIndex={-1} aria-hidden="true" data-testid="chat-file-input"
          onChange={(e) => {
            addAttachments(target.path, Array.from(e.currentTarget.files ?? []));
            e.currentTarget.value = "";
            field.current?.focus({ preventScroll: true });
          }} />
        <button className="chat-attach" onClick={() => picker.current?.click()} disabled={v.sending || !!v.attachOff}
          aria-label={v.attachOff ? `Attach files: ${v.attachOff}` : "Attach files"} title={v.attachOff ?? "Attach files (or paste or drop them here)"}
          data-testid="chat-attach">
          <Icons.Paperclip size={15} />
        </button>
        <label htmlFor={`${ids}-f`} className="sr-only">Message {agent}</label>
        <textarea id={`${ids}-f`} ref={field} className="chat-field" rows={1} value={text}
          placeholder={props.waiting ? "Answer the prompt to continue" : dropping ? "Drop to attach" : `Message ${agent}, or / for a command`}
          aria-describedby={`${ids}-l`} aria-invalid={problem ? true : undefined}
          role={menuOn ? "combobox" : undefined} aria-expanded={menuOn ? rows.length > 0 : undefined}
          aria-controls={menuOn ? `${ids}-m` : undefined} aria-autocomplete={menuOn ? "list" : undefined}
          aria-activedescendant={menuOn && rows[hi] ? `${ids}-m-${hi}` : undefined}
          autoCorrect="off" autoCapitalize="off" autoComplete="off" spellCheck={false}
          onChange={(e) => setDraft(target.path, e.currentTarget.value)}
          onPaste={(e) => {
            // An image or a file on the clipboard is an attachment; text
            // pastes as it always has.
            const pasted = Array.from(e.clipboardData?.files ?? []);
            if (!pasted.length || v.attachOff) return;
            e.preventDefault();
            addAttachments(target.path, pasted);
          }}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            // The menu has the arrows, Tab and Enter while it shows rows.
            if (menuOn && rows.length) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                setHi((h) => (h + (e.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length);
                return;
              }
              if ((e.key === "Enter" && !e.shiftKey && !e.altKey) || (e.key === "Tab" && !e.shiftKey)) {
                e.preventDefault();
                pick(rows[Math.min(hi, rows.length - 1)], e.key === "Tab" ? "tab" : "enter");
                return;
              }
            }
            if (e.key !== "Enter") return;
            // A message is one line: Enter sends it, and no key adds a break.
            e.preventDefault();
            if (!e.shiftKey && !e.altKey) send();
          }}
          data-testid="chat-field" />
        {props.busy && (
          <button className="chat-stop" onClick={stop} disabled={v.stopping} aria-keyshortcuts="Escape"
            aria-label={`Stop ${agent}’s turn`} title={`Stop ${agent}’s turn (Esc)`} data-testid="chat-stop">
            {v.stopping ? <span className="chat-spin" aria-hidden="true" /> : <span className="chat-stop-sq" aria-hidden="true" />}
          </button>
        )}
        <button className="chat-send" onClick={send} disabled={!canSend} aria-label={`Send to ${agent}`} title={`Send to ${agent}`} data-testid="chat-send">
          {v.sending ? <span className="chat-spin" aria-hidden="true" /> : <span aria-hidden="true">↑</span>}
        </button>
      </div>
      <div className="chat-line-row">
        <p id={`${ids}-l`} className={"chat-line" + (alert ? " alert" : "")} role={alert ? "alert" : undefined} data-testid="chat-line">
          {line}
          {joinable && !v.sendWords && <> <button className="chat-linkbtn" onClick={() => setDraft(target.path, text.replace(/\s*[\r\n\t]+\s*/g, " ").trim())}>Join into one line</button></>}
          {said?.terminal && !v.sending && !v.stopping && <> <button className="chat-linkbtn" onClick={props.onTerminal}>Open the terminal</button></>}
        </p>
        <button className="chat-linkbtn chat-refresh" onClick={props.onRefresh} disabled={props.loading} title="Read the transcript and the prompt again">
          {props.loading ? "Reading…" : "Refresh"}
        </button>
      </div>
    </div>
  );
}

/** The composer's `/` menu: the agent's commands that match what is typed,
 *  each with its description and where it comes from. The field keeps the
 *  keyboard (arrows, Tab, Enter; Esc closes it through `useEscape`); a click
 *  is a pick. */
function SlashMenu(props: {
  id: string; rows: SlashCommand[]; hi: number; loading: boolean; error: string | null;
  onHover: (i: number) => void; onPick: (c: SlashCommand) => void;
}) {
  const list = useRef<HTMLUListElement>(null);
  useEffect(() => {
    list.current?.querySelector(`[data-i="${props.hi}"]`)?.scrollIntoView({ block: "nearest" });
  }, [props.hi]);
  if (props.loading || props.error || !props.rows.length) {
    return (
      <div className="chat-slash chat-slash-empty" data-testid="chat-slash">
        {props.loading ? "Reading the agent’s commands…" : props.error ?? "No command matches."}
      </div>
    );
  }
  return (
    <ul ref={list} id={props.id} className="chat-slash" role="listbox" aria-label="Commands" data-testid="chat-slash">
      {props.rows.map((c, i) => (
        <li key={c.name} id={`${props.id}-${i}`} data-i={i} role="option" aria-selected={i === props.hi}
          className={"chat-slash-row" + (i === props.hi ? " on" : "")} data-testid={`chat-slash-${c.name}`}
          onMouseDown={(e) => e.preventDefault()} onMouseEnter={() => props.onHover(i)} onClick={() => props.onPick(c)}>
          <span className="chat-slash-name">/{c.name}{c.hint ? <span className="chat-slash-hint"> {c.hint}</span> : null}</span>
          <span className="chat-slash-desc" title={c.description}>{c.description}</span>
          <span className="chat-slash-tags">
            {c.tier === "sensitive" && <span className="chat-slash-tag sensitive" title={c.why ?? "Sensitive"}>Sensitive</span>}
            <span className="chat-slash-tag">{sourceWord(c)}</span>
          </span>
        </li>
      ))}
    </ul>
  );
}

/** The agent cell's Chat | Terminal switch. Two buttons, not a toggle: the
 *  one that is on says which view this is. */
export function AgentViewSwitch(props: { chat: boolean; onChat: (on: boolean) => void }) {
  return (
    <span className="agent-view" role="group" aria-label="View" data-testid="agent-view">
      <button className={"agent-view-b" + (props.chat ? " on" : "")} aria-pressed={props.chat}
        onClick={() => props.onChat(true)} title="Show the agent’s turns as a chat" data-testid="agent-view-chat">
        <Icons.MessageSquare size={12} /> Chat
      </button>
      <button className={"agent-view-b" + (props.chat ? "" : " on")} aria-pressed={!props.chat}
        onClick={() => props.onChat(false)} title="Show the agent’s terminal" data-testid="agent-view-terminal">
        <Icons.SquareTerminal size={12} /> Terminal
      </button>
    </span>
  );
}
