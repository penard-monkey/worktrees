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
  activityMoved, boxesOf, bytes, callsSay, callsWords, cardDigits, cardKey, cardTakesFocus, choiceOpen, clockMs, clockOf,
  dayLabel, draftProblem, fieldWords, foldCalls, itemsOf, keyFocusOf, lineDiff, loadPrompt, loadTurns, permissionAnswerWords,
  promptBody, questionAnswerWords, selectChoice, sendAnswer, sendDraft, sendMany, setAnswerText, setBoxText, setDraft,
  splitAtRule, stopTurn, ticksOf, toggleTick, turnsUnsupported, underRoot, useChat, SEND_MAX,
  type AskedStep, type Item, type PromptChoice, type PromptDetail, type ReadablePrompt, type Target, type ToolCall,
} from "./chat";
// What a screen reader is told (VoiceOver): its own line, so the lanes' imports merge apart.
import { announcement, gather, hear, promptKeyOf, type Bundle, type ChatView, type Heard } from "./chat";
import { HARNESS_LABEL, type Harness } from "./harness";
import * as Icons from "./icons";
import { Markdown } from "./markdown";
import { useDocPathLinks } from "./useDocPathLinks";
import { escapeDepth } from "./useEscape";
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
  const agent = HARNESS_LABEL[provider];
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
  const items = data ? itemsOf(data.turns, v.answered, v.pending) : [];
  const replies = items.filter((it) => it.k === "reply").length;
  const promptKey = promptKeyOf(prompt);
  const { toBottom, fresh, dismiss } = useStickToBottom(scrollRef, items.length, replies, !!data, promptKey);
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
          {" "}Tool output, diffs and slash commands are in the{" "}
          <button className="chat-linkbtn" onClick={props.onTerminal}>terminal</button>.
        </p>
        {items.length === 0 && !prompt && <p className="chat-note" data-testid="chat-empty">No messages in this session yet.</p>}
        <ol className="chat-list" data-testid="chat-list">
          {items.map((it) => <ChatItem key={it.key} it={it} agent={agent} root={path} onLink={onLink} pathLinks={pathLinks} />)}
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
    <div className="chat" data-testid="chat" onKeyDown={onKeyDown}>
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
        busy={busy} onTerminal={props.onTerminal} onRefresh={refresh} loading={v.turns.loading || v.prompt.loading} />
      {menu}
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

/** Open at the newest; follow new content only when already at the bottom,
 *  else offer "New messages". A NEW prompt is always brought into view, and
 *  one taller than the view opens at its top: what it asks is read before
 *  its choices, never scrolled past to reach them. */
function useStickToBottom(ref: RefObject<HTMLDivElement | null>, count: number, replies: number, loaded: boolean, promptKey: string | null) {
  const atBottom = useRef(true);
  const seen = useRef<number | null>(null);
  const seenReplies = useRef(0);
  const seenPrompt = useRef<string | null>(null);
  const [fresh, setFresh] = useState(false);
  const toBottom = () => { const el = ref.current; if (el) el.scrollTop = el.scrollHeight; };
  const toPrompt = () => {
    const el = ref.current;
    const card = el?.querySelector<HTMLElement>(".chat-card");
    if (!el || !card) return toBottom();
    if (card.offsetHeight <= el.clientHeight - 16) return toBottom();
    el.scrollTop += card.getBoundingClientRect().top - el.getBoundingClientRect().top - 8;
    atBottom.current = false;
  };
  useLayoutEffect(() => {
    if (!loaded || promptKey === seenPrompt.current) return;
    seenPrompt.current = promptKey;
    if (promptKey !== null) toPrompt();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [promptKey, loaded]);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const f = () => {
      atBottom.current = el.scrollHeight - el.clientHeight - el.scrollTop <= 48;
      if (atBottom.current) setFresh(false);
    };
    el.addEventListener("scroll", f, { passive: true });
    return () => el.removeEventListener("scroll", f);
  }, [ref]);
  useLayoutEffect(() => {
    if (!loaded) return;
    if (seen.current === null) {
      seen.current = count;
      seenReplies.current = replies;
      if (promptKey !== null) toPrompt();
      else toBottom();
      return;
    }
    if (count > seen.current) {
      if (atBottom.current) toBottom();
      else if (replies > seenReplies.current) setFresh(true);
    }
    seen.current = count;
    seenReplies.current = replies;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [count, replies, loaded]);
  // The column growing (a card, a wrapped composer) keeps the bottom in view.
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => { if (atBottom.current && seen.current !== null) toBottom(); });
    ro.observe(el);
    if (el.firstElementChild) ro.observe(el.firstElementChild);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);
  return { toBottom, fresh, dismiss: () => setFresh(false) };
}

/** A turn's tool calls. From a host that says what each did: a row per
 *  call, its name, its line (the place's root taken off its paths, the whole
 *  line in the title) and a failed call's dot AND word, the first few shown
 *  and the rest behind "+N more", which names a failure it hides. From an
 *  older host: the names, as chips. Text only: a summary is transcript text. */
function ToolCalls(props: { calls: ToolCall[]; root: string }) {
  const [open, setOpen] = useState(false);
  const { calls } = props;
  if (!callsSay(calls)) {
    return (
      <ul className="chat-tools" title="Tools it called. Their output is in the terminal.">
        {calls.map((c, i) => <li key={i} className="chat-tool">{c.name}</li>)}
      </ul>
    );
  }
  const { shown, hidden, hiddenFailed, folds } = foldCalls(calls, open);
  return (
    <ul className="chat-calls" title="Tools it called. Their output is in the terminal." data-testid="chat-calls">
      {shown.map((c, i) => (
        <li key={i} className={"chat-call" + (c.failed ? " failed" : "")} data-testid="chat-call">
          <span className="chat-tool chat-call-name">{c.name}</span>
          {c.summary && <span className="chat-call-sum" title={c.summary}>{underRoot(c.summary, props.root)}</span>}
          {c.failed && <span className="chat-call-failed"><span className="chat-call-dot" aria-hidden="true" />failed</span>}
        </li>
      ))}
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

function ChatItem(props: { it: Item; agent: string; root: string; onLink: (href: string) => void; pathLinks: ReturnType<typeof useDocPathLinks>["pathLinks"] }) {
  const { it, agent } = props;
  switch (it.k) {
    case "sep":
      return <li className="chat-sep" title={new Date(it.at * 1000).toLocaleString()}>{dayLabel(it.at)}</li>;
    case "earlier":
      return <li className="chat-note">This turn began before the transcript window: its message isn’t shown.</li>;
    case "user":
      return (
        <li className="chat-row chat-row-user">
          <div className="chat-bubble chat-bubble-user" role="group"
            aria-label={`You${it.at ? `, ${clockOf(it.at)}` : ""}`} data-testid="chat-user">
            <Markdown src={it.text} onLink={props.onLink} />
          </div>
        </li>
      );
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
            {it.calls.length > 0 && <ToolCalls calls={it.calls} root={props.root} />}
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
          You answered “{it.a.label}”{it.a.note ? <> with a note: “{it.a.note}”</> : null} · {clockMs(it.a.at)}{it.a.confirmed ? "" : " · not confirmed"}
        </li>
      );
    case "pending":
      return (
        <li className="chat-row chat-row-user">
          <div className="chat-bubble chat-bubble-user chat-bubble-pending" data-testid="chat-pending">
            <p>{it.p.text}</p>
          </div>
          <p className="chat-status">{it.p.queued ? `Queued behind ${agent}’s running turn` : `Delivered · waiting for ${agent}`}</p>
        </li>
      );
  }
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
  const selected = p.choices.find((c) => c.n === v.selected) ?? null;
  const way = selected ? choiceOpen(selected) : null;
  const typed = way && way.how !== "plain" ? v.answerText : "";
  const typedProblem = way && way.how !== "plain" ? draftProblem(typed, way.how === "field" ? "answer" : "note") : null;
  const canAnswer = !!selected && !!way?.open && !typedProblem && !(way.how === "field" && !typed.trim()) && !v.answering;
  const freeOn = !!freeBox && ticks.includes(freeBox.n);
  const boxProblem = freeOn ? (v.boxText.trim() ? draftProblem(v.boxText, "answer") : "Type your own answer in its box, or untick it.") : null;
  const canSendMany = !!p.multi && !selected && ticks.length > 0 && !boxProblem && !v.answering;
  const kind = p.detail?.tool === "ExitPlanMode" ? "Plan" : p.kind === "question" ? "Question" : "Permission";
  const answer = () => { if (canAnswer) void sendAnswer(target, agent); };
  const many = () => { if (canSendMany) void sendMany(target); };
  // Enter's send: what the Send button on screen runs, behind its own enabled state.
  const send = selected && way ? { on: canAnswer, run: answer } : p.multi ? { on: canSendMany, run: many } : null;
  const digits = cardDigits(p);
  const quote = (s: string) => `“${s.length > 40 ? s.slice(0, 39) + "…" : s}”`;

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
        <b id={`${ids}-h`}>{agent} needs your answer</b>
        <span className="chat-kind">{kind}</span>
      </div>
      {p.detail && <PromptDetail d={p.detail} root={target.path} agent={agent} onLink={props.onLink} />}
      {above.length > 0 && (
        <details className="chat-screen" data-testid="chat-prompt-screen">
          <summary>The part the terminal shows ({above.length} rows)</summary>
          <pre className="chat-prompt-lines">{above.join("\n")}</pre>
        </details>
      )}
      <pre className="chat-prompt-lines" data-testid="chat-prompt-lines">{question.join("\n")}</pre>
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
        <div className="chat-choices" role="group" aria-label={p.multi ? "Or instead" : "Choices"}>
          {others.map((c) => (
            <ChoiceRow key={c.n} c={c} on={v.selected === c.n} busy={v.answering} path={target.path}
              harness={p.harness} agent={agent} text={v.answerText} problem={v.selected === c.n ? typedProblem : null} onEnter={answer}
              focusToken={keyPick?.n === c.n ? keyPick.t : 0} />
          ))}
        </div>
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
              {v.answering ? "Answering…"
                : way.how === "field" ? (typed.trim() ? `Send answer: ${quote(typed)}` : "Type an answer to send")
                : way.how === "amend" && typed.trim() ? `Send ${quote(selected.label)} with the note`
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
          <span className="chat-hint">Pick a choice, then send it.</span>
        )}
        {digits && !v.answering && <span className="chat-hint" data-testid="chat-keys-hint">Press {digits}, then Enter.</span>}
        <button className="chat-linkbtn chat-foot-end" onClick={props.onTerminal}>Answer in the terminal</button>
      </div>
    </div>
  );
}

/** One choice of a prompt: a button that selects it and, once selected, the
 *  text it takes — a field's answer, required, or an optional note. */
function ChoiceRow(props: {
  c: PromptChoice; on: boolean; busy: boolean; path: string; harness: string; agent: string;
  text: string; problem: string | null; onEnter: () => void;
  /** Moves when a DIGIT selects this choice: its text takes the keyboard,
   *  a note's too, since the next keys are typed there (Enter sends). */
  focusToken: number;
}) {
  const { c, on } = props;
  const { open, how, note } = choiceOpen(c);
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
  onTerminal: () => void; onRefresh: () => void; loading: boolean;
}) {
  const { target, agent } = props;
  const v = useChat(target.path);
  const field = props.fieldRef;
  const ids = useId();
  const text = v.draft;
  const problem = draftProblem(text);
  const n = bytes(text);

  // Switching to Chat is switching to typing here: the hidden terminal has
  // just given the keyboard up (App's `setChatView`). Never over a prompt card
  // that has taken it (ChatPane): StrictMode runs this twice.
  useEffect(() => {
    if (!document.activeElement?.closest(".chat-prompt")) field.current?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => { if (props.followUp) field.current?.focus({ preventScroll: true }); }, [props.followUp]);

  // Grow with the text, to six lines; a long line wraps, it never scrolls sideways.
  useLayoutEffect(() => {
    const el = field.current;
    if (!el) return;
    el.style.height = "auto";
    const cs = getComputedStyle(el);
    const line = parseFloat(cs.lineHeight) || 20;
    const pad = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
    const border = parseFloat(cs.borderTopWidth) + parseFloat(cs.borderBottomWidth);
    el.style.height = `${Math.min(el.scrollHeight, line * 6 + pad) + border}px`;
  }, [text]);

  const canSend = !!text.trim() && !problem && !props.waiting && !v.sending;
  const send = () => { if (canSend) void sendDraft(target, agent); };
  // Only a line break or a tab is wrong: offer to join, by hand, never silently.
  const joinable = !!problem && /[\n\r\t]/.test(text) && !draftProblem(text.replace(/\s*[\r\n\t]+\s*/g, " ").trim());

  const stop = () => { if (!v.stopping) void stopTurn(target, agent); };
  const said = v.sendWords ?? v.stopWords;

  let line: React.ReactNode;
  let alert = false;
  if (v.sending) line = "Sending…";
  else if (v.stopping) line = `Stopping ${agent}’s turn…`;
  else if (v.sendWords) { line = v.sendWords.line; alert = true; }
  // A stop's words: a refusal or an unconfirmed stop is an alert; "Stopped." is not.
  else if (v.stopWords) { line = v.stopWords.line; alert = !!(v.stopWords.terminal || v.stopWords.prompt); }
  else if (problem) { line = problem; alert = true; }
  else if (props.waiting) line = `Messages can’t be sent while a prompt is waiting. Answer it ${props.waiting === "above" ? "above" : "in the terminal"}.`;
  else if (props.followUp && !text.trim()) line = props.followUp;
  else line = n >= COUNT_FROM ? `One line · ${n.toLocaleString()} of ${SEND_MAX.toLocaleString()} bytes` : "One line · Enter sends";

  return (
    <div className="chat-composer" data-testid="chat-composer">
      {props.paneDraft && !props.waiting && (
        <p className="chat-paneDraft" data-testid="chat-pane-draft">
          The terminal’s input holds unsent text (“{props.paneDraft.text.length > 60 ? props.paneDraft.text.slice(0, 59) + "…" : props.paneDraft.text}”).
          {" "}{agent} takes no message until it is sent or cleared{" "}
          <button className="chat-linkbtn" onClick={props.onTerminal}>in the terminal</button>.
        </p>
      )}
      <div className="chat-composer-row">
        <label htmlFor={`${ids}-f`} className="sr-only">Message {agent}</label>
        <textarea id={`${ids}-f`} ref={field} className="chat-field" rows={1} value={text}
          placeholder={props.waiting ? "Answer the prompt to continue" : `Message ${agent}`}
          aria-describedby={`${ids}-l`} aria-invalid={problem ? true : undefined}
          autoCorrect="off" autoCapitalize="off" autoComplete="off" spellCheck={false}
          onChange={(e) => setDraft(target.path, e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter" || e.nativeEvent.isComposing) return;
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
