// The chat view of a place's agent, beside its terminal (docs/proposals/
// chat-session-view.md). A per-place Chat | Terminal switch in the agent cell
// picks it; the terminal stays MOUNTED underneath, hidden and inert, so the
// way back is one click with its scrollback intact (the reading overlay's
// rule). State and rules are chat.ts; this file draws them.
//
// What it is NOT: a terminal. No tool output, no diffs, no slash commands, no
// Esc. Each of those says so where it would be wanted, and the terminal is
// named as the place to do it.

import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type RefObject } from "react";
import {
  bytes, choiceOpen, clockMs, clockOf, dayLabel, draftProblem, itemsOf, loadPrompt, loadTurns, promptBody,
  selectChoice, sendAnswer, sendDraft, setDraft, turnsUnsupported, useChat, SEND_MAX,
  type Item, type ReadablePrompt, type Target,
} from "./chat";
import { HARNESS_LABEL, type Harness } from "./harness";
import * as Icons from "./icons";
import { Markdown } from "./markdown";
import { useDocPathLinks } from "./useDocPathLinks";
import "./ChatPane.css";

/** While the agent works, the one visible chat re-reads its turns this often:
 *  `sessions:busy` moves only when a turn starts or ends, and a turn can run
 *  for many minutes. Nothing polls while idle, waiting, or hidden. */
const BUSY_POLL_MS = 10_000;
/** The byte count appears under the field from here. */
const COUNT_FROM = 3500;

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
  const working = activity === "busy" && !prompt;

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

  const { pathLinks, menu } = useDocPathLinks(path, null, props.reloadToken, props.onOpenPath);
  const onLink = useCallback((href: string) => {
    if (/^https?:/i.test(href)) openUrl(href).catch((e) => props.onError(`open ${href}: ${String(e)}`));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const data = v.turns.data;
  const items = data ? itemsOf(data.turns, v.answered, v.pending) : [];
  const replies = items.filter((it) => it.k === "reply").length;
  const { toBottom, fresh, dismiss } = useStickToBottom(scrollRef, items.length, replies, !!data, !!prompt);

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
          {items.map((it) => <ChatItem key={it.key} it={it} agent={agent} onLink={onLink} pathLinks={pathLinks} />)}
        </ol>
      </>
    );
  }

  return (
    <div className="chat" data-testid="chat">
      <div className="chat-scrollwrap">
      <div className="chat-scroll" ref={scrollRef}>
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
              ? <PromptCard p={prompt} target={target} agent={agent} onTerminal={props.onTerminal} />
              : (
                <div className="chat-card" data-testid="chat-prompt-unreadable">
                  <div className="chat-card-head"><span className="chat-card-dot" aria-hidden="true" /><b>{agent} needs input</b></div>
                  <p>This prompt can’t be read whole here (taller than its pane, or one this app doesn’t know). Answer it in the terminal.</p>
                  <button className="ctrl sm with-icon" onClick={props.onTerminal}><Icons.SquareTerminal size={13} /> Open the terminal</button>
                </div>
              )
          )}
        </div>
      </div>
      {fresh && (
        <button className="chat-new" onClick={() => { dismiss(); toBottom(); }} data-testid="chat-new">New messages ↓</button>
      )}
      </div>
      <Composer target={target} agent={agent} waiting={!!prompt} paneDraft={props.paneDraft}
        onTerminal={props.onTerminal} onRefresh={refresh} loading={v.turns.loading || v.prompt.loading} />
      {menu}
    </div>
  );
}

/** Open at the newest; follow new content only when already at the bottom,
 *  else offer "New messages". */
function useStickToBottom(ref: RefObject<HTMLDivElement | null>, count: number, replies: number, loaded: boolean, prompt: boolean) {
  const atBottom = useRef(true);
  const seen = useRef<number | null>(null);
  const seenReplies = useRef(0);
  const [fresh, setFresh] = useState(false);
  const toBottom = () => { const el = ref.current; if (el) el.scrollTop = el.scrollHeight; };
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
      toBottom();
      return;
    }
    if (count > seen.current || prompt) {
      if (atBottom.current) toBottom();
      else if (replies > seenReplies.current) setFresh(true);
    }
    seen.current = count;
    seenReplies.current = replies;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [count, replies, loaded, prompt]);
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

const toolsWords = (tools: string[]) => {
  const names = [...new Set(tools)];
  if (!names.length) return "";
  return names.length === 1 ? `Used ${names[0]}.` : `Used ${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}.`;
};

function ChatItem(props: { it: Item; agent: string; onLink: (href: string) => void; pathLinks: ReturnType<typeof useDocPathLinks>["pathLinks"] }) {
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
            aria-label={`${agent}${it.at ? `, ${clockOf(it.at)}` : ""}${it.tools.length ? `. ${toolsWords(it.tools)}` : ""}`}
            data-testid="chat-agent">
            {(it.text || it.truncated) && (
              <div className="chat-bubble chat-bubble-agent">
                {it.text && <Markdown src={it.text} onLink={props.onLink} pathLinks={props.pathLinks} />}
                {it.truncated && <p className="chat-bubble-note">Shortened by the host: the full reply is in the terminal.</p>}
              </div>
            )}
            {it.tools.length > 0 && (
              <ul className="chat-tools" title="Tools it called. Their output is in the terminal.">
                {it.tools.map((tool, i) => <li key={i} className="chat-tool">{tool}</li>)}
              </ul>
            )}
          </div>
        </li>
      );
    case "answered":
      return (
        <li className="chat-note chat-answered" data-testid="chat-answered">
          You answered “{it.a.label}” · {clockMs(it.a.at)}{it.a.confirmed ? "" : " · not confirmed"}
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

/** A waiting prompt, whole, with its own choices. A click selects; "Send
 *  answer" sends — two acts, because an answer can run a command. */
function PromptCard(props: { p: ReadablePrompt; target: Target; agent: string; onTerminal: () => void }) {
  const { p, target, agent } = props;
  const v = useChat(target.path);
  const ids = useId();
  const selected = p.choices.find((c) => c.n === v.selected) ?? null;
  return (
    <div className="chat-card chat-prompt" role="group" aria-labelledby={`${ids}-h`} data-testid="chat-prompt">
      <div className="chat-card-head">
        <span className="chat-card-dot" aria-hidden="true" />
        <b id={`${ids}-h`}>{agent} needs your answer</b>
        <span className="chat-kind">{p.kind === "question" ? "Question" : "Permission"}</span>
      </div>
      <pre className="chat-prompt-lines" data-testid="chat-prompt-lines">{promptBody(p).join("\n")}</pre>
      <div className="chat-choices" role="group" aria-label="Choices">
        {p.choices.map((c) => {
          const { open, note } = choiceOpen(c);
          const on = v.selected === c.n;
          return (
            <button key={c.n} className={"chat-choice" + (on ? " on" : "") + (open ? "" : " closed")}
              aria-pressed={open ? on : undefined} disabled={!open || v.answering}
              onClick={() => selectChoice(target.path, on ? null : c.n)} data-testid={`chat-choice-${c.n}`}>
              {/* The grid is an inner span: WebKit shrink-wraps a <button>
                  that is itself the container (AGENTS.md, WebKit probe). */}
              <span className="chat-choice-in">
                <span className="chat-choice-n" aria-hidden="true">{c.n}</span>
                <span className="chat-choice-text">
                  <span className="chat-choice-label">{c.label}</span>
                  {c.more?.map((m, i) => <span key={i} className="chat-choice-more">{m}</span>)}
                  {note && <span className="chat-choice-note">{note}</span>}
                </span>
              </span>
            </button>
          );
        })}
      </div>
      {v.answerLine && <p className="chat-alert" role="alert" data-testid="chat-answer-line">{v.answerLine}</p>}
      <div className="chat-card-foot">
        {selected ? (
          <>
            <button className="enter-btn sm" disabled={v.answering} onClick={() => void sendAnswer(target)} data-testid="chat-send-answer">
              {v.answering ? "Answering…" : `Send answer: “${selected.label.length > 40 ? selected.label.slice(0, 39) + "…" : selected.label}”`}
            </button>
            <button className="ctrl sm" disabled={v.answering} onClick={() => selectChoice(target.path, null)}>Cancel</button>
          </>
        ) : (
          <span className="chat-hint">Pick a choice, then send it.</span>
        )}
        <button className="chat-linkbtn chat-foot-end" onClick={props.onTerminal}>Answer in the terminal</button>
      </div>
    </div>
  );
}

function Composer(props: {
  target: Target; agent: string; waiting: boolean; paneDraft: { text: string; queued: boolean } | null;
  onTerminal: () => void; onRefresh: () => void; loading: boolean;
}) {
  const { target, agent } = props;
  const v = useChat(target.path);
  const field = useRef<HTMLTextAreaElement>(null);
  const ids = useId();
  const text = v.draft;
  const problem = draftProblem(text);
  const n = bytes(text);

  // Switching to Chat is switching to typing here: the hidden terminal has
  // just given the keyboard up (App's `setChatView`).
  useEffect(() => { field.current?.focus({ preventScroll: true }); }, []);

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

  let line: React.ReactNode;
  let alert = false;
  if (v.sending) line = "Sending…";
  else if (v.sendWords) { line = v.sendWords.line; alert = true; }
  else if (problem) { line = problem; alert = true; }
  else if (props.waiting) line = "Messages can’t be sent while a prompt is waiting. Answer it above.";
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
        <button className="chat-send" onClick={send} disabled={!canSend} aria-label={`Send to ${agent}`} title={`Send to ${agent}`} data-testid="chat-send">
          {v.sending ? <span className="chat-spin" aria-hidden="true" /> : <span aria-hidden="true">↑</span>}
        </button>
      </div>
      <div className="chat-line-row">
        <p id={`${ids}-l`} className={"chat-line" + (alert ? " alert" : "")} role={alert ? "alert" : undefined} data-testid="chat-line">
          {line}
          {joinable && !v.sendWords && <> <button className="chat-linkbtn" onClick={() => setDraft(target.path, text.replace(/\s*[\r\n\t]+\s*/g, " ").trim())}>Join into one line</button></>}
          {v.sendWords?.terminal && <> <button className="chat-linkbtn" onClick={props.onTerminal}>Open the terminal</button></>}
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
