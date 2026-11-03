// The chat's scroll (ChatPane, docs/proposals/chat-session-view.md §7): open
// at the newest, follow whatever arrives while the reader is at the bottom,
// and hold still for a reader who has scrolled up. `chatscroll-check.mjs`
// runs this file's real source; `chat-scroll-browser.mjs` drives it in
// Chromium and WebKit.
//
// Why it is built this way (2026-11-02, "when I get a response the view
// jumps up to roughly 3 messages ago"; measured in WebKit):
//   - A reply rarely grows the item COUNT. `agent_turns` answers the latest
//     20 turns, so a new turn pushes the oldest off the top, and a pending
//     message is SWAPPED for the transcript's copy. So this follows on ANY
//     commit while the reader is at the bottom, not on a count.
//   - WebKit anchors scrolling as Chromium does (the system WKWebView on
//     macOS 27.0 included): content removed above the view moves `scrollTop`,
//     and the `scroll` event that move fires arrives BEFORE the
//     ResizeObserver's follow. The old hook took that event for the reader
//     scrolling up and stopped following, 704 px from the bottom. Here native
//     anchoring is off (`overflow-anchor: none` on `.chat-scroll`), this file
//     does the anchoring itself, and a scroll event counts only when the
//     position moved since this file last saw it, so a scroll it made itself
//     never reads as the reader's.
//   - "Was the reader at the bottom?" is decided before the commit changes
//     the DOM (`take`, from `BeforeCommit`), never from what the commit did.
import { Component, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";

/** This close to the bottom counts as at it: a reader a line up still follows. */
export const NEAR = 48;

/** A row in view, at its CONTENT position (its top plus `scrollTop`), which
 *  scrolling does not change and content above it does. */
type Mark = { node: Element; pos: number };
/** The scroller as this file last saw it. */
type Seen = { st: number; fb: number; marks: Mark[] };

const fromBottom = (el: HTMLElement) => el.scrollHeight - el.clientHeight - el.scrollTop;

/** What a reader can be reading: the column's children, with the list's
 *  items in place of the list. React keeps a row's node for as long as its
 *  key lives (`itemsOf` keys by turn time), so a node is the row's identity. */
function rowsOf(el: HTMLElement): Element[] {
  const out: Element[] = [];
  for (const c of Array.from(el.firstElementChild?.children ?? [])) {
    if (c.tagName === "OL") out.push(...Array.from(c.children));
    else out.push(c);
  }
  return out;
}

/** Where the scroller is, and where the rows in view are. */
function look(el: HTMLElement): Seen {
  const box = el.getBoundingClientRect();
  const st = el.scrollTop;
  const marks: Mark[] = [];
  for (const row of rowsOf(el)) {
    const r = row.getBoundingClientRect();
    if (r.bottom <= box.top) continue;
    if (r.top >= box.bottom) break;
    marks.push({ node: row, pos: r.top - box.top + st });
  }
  return { st, fb: fromBottom(el), marks };
}

/** Put the first row in view that is still there back where it was, so
 *  rows leaving the top or growing above it do not move what is read. */
function hold(el: HTMLElement, seen: Seen) {
  const box = el.getBoundingClientRect();
  for (const m of seen.marks) {
    if (!m.node.isConnected) continue;
    const d = m.node.getBoundingClientRect().top - box.top + el.scrollTop - m.pos;
    if (Math.abs(d) >= 1) el.scrollTop = el.scrollTop + d;
    return;
  }
}

/** Calls `take` as React is about to change the DOM, in every commit it is
 *  rendered in. `getSnapshotBeforeUpdate` is React's one moment for reading
 *  the DOM as the reader saw it before a commit, and no hook has it. */
export class BeforeCommit extends Component<{ take: () => void }> {
  getSnapshotBeforeUpdate() { this.props.take(); return null; }
  componentDidUpdate() {}
  render() { return null; }
}

/** Open at the newest; follow any change while at the bottom, else hold the
 *  reader's rows still and offer "New messages" for a reply that arrived
 *  below. A NEW prompt is always brought into view, and one taller than the
 *  view opens at its top: what it asks is read before its choices, never
 *  scrolled past to reach them. `lastReply` is the newest reply's key.
 *  Render `<BeforeCommit take={take} />` in the same component. */
export function useStickToBottom(ref: RefObject<HTMLDivElement | null>, loaded: boolean, promptKey: string | null, lastReply: string | null) {
  /** Follow from the bottom: the reader's choice, by where they scrolled. */
  const pinned = useRef(true);
  const seen = useRef<Seen | null>(null);
  const opened = useRef(false);
  const seenPrompt = useRef<string | null>(null);
  const seenReply = useRef<string | null>(null);
  const freshNow = useRef(false);
  const [fresh, setFreshState] = useState(false);
  const setFresh = (v: boolean) => {
    if (freshNow.current === v) return;
    freshNow.current = v;
    setFreshState(v);
  };
  const note = (el: HTMLElement) => { seen.current = look(el); };
  const toBottom = () => {
    const el = ref.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    pinned.current = true;
    setFresh(false);
    note(el);
  };
  const toPrompt = () => {
    const el = ref.current;
    const card = el?.querySelector<HTMLElement>(".chat-card");
    if (!el || !card || card.offsetHeight <= el.clientHeight - 16) return toBottom();
    el.scrollTop += card.getBoundingClientRect().top - el.getBoundingClientRect().top - 8;
    pinned.current = false;
    note(el);
  };
  /** Not following: hold the rows in view, and follow again from a bottom
   *  that came to the reader (what was below them went away). */
  const stay = (el: HTMLElement, before: Seen) => {
    hold(el, before);
    const fb = fromBottom(el);
    if (fb <= NEAR && fb < before.fb - 0.5) {
      pinned.current = true;
      setFresh(false);
    }
    note(el);
  };
  /** The reader moved since this file last looked: where they are now
   *  decides. A scroll this file made was looked at as it was made. */
  const moved = (el: HTMLElement) => {
    const s = seen.current;
    if (s && Math.abs(el.scrollTop - s.st) < 1) return false;
    pinned.current = fromBottom(el) <= NEAR;
    return true;
  };

  // Before the commit: a scroll whose event is still on its way counts, and
  // the rows in view are where the reader left them.
  const take = () => {
    const el = ref.current;
    if (!el || !opened.current) return;
    moved(el);
    note(el);
  };

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const f = () => {
      if (!opened.current || !moved(el)) return;
      if (pinned.current) setFresh(false);
      note(el);
    };
    el.addEventListener("scroll", f, { passive: true });
    return () => el.removeEventListener("scroll", f);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ref]);

  // Every commit: whatever changed (rows added, swapped or gone, a reply
  // grown, the window slid), follow from the bottom or hold still.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (!loaded) { note(el); return; }
    if (!opened.current) {
      opened.current = true;
      seenPrompt.current = promptKey;
      seenReply.current = lastReply;
      if (promptKey !== null) toPrompt();
      else toBottom();
      return;
    }
    const before = seen.current;
    if (pinned.current) toBottom();
    else {
      if (lastReply !== null && lastReply !== seenReply.current) setFresh(true);
      if (before) stay(el, before);
      else note(el);
    }
    seenReply.current = lastReply;
    if (promptKey !== seenPrompt.current) {
      seenPrompt.current = promptKey;
      if (promptKey !== null) toPrompt();
    }
  });

  // Size changes no commit made (a wrapped composer, the window, a fold
  // opened): follow from the bottom, or hold the rows in view.
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      if (!opened.current) return;
      if (pinned.current) toBottom();
      else if (seen.current) stay(el, seen.current);
    });
    ro.observe(el);
    if (el.firstElementChild) ro.observe(el.firstElementChild);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ref]);

  return { toBottom, fresh, dismiss: () => setFresh(false), take };
}
