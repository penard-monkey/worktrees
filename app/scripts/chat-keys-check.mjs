// chat-keys-check — the chat view's prompt card answers from the keyboard the
// way a click does (docs/proposals/chat-session-view.md §7): a digit 1–9
// selects that choice, ticks that box, or selects a choice and focuses the
// text it takes; Enter is the card's Send button, only while that button is
// enabled. Never while an answer is in flight, a modal is up (`escapeDepth()`),
// a modifier is held (⌘1–9 is App's nav chord), an IME is composing, or the
// keyboard is in a text field or a terminal (the dock shell keeps its digits).
// And a card that appears takes the keyboard from an EMPTY composer, once per
// prompt, and from nothing else; typing at the card (a letter, or any key in
// the first second after it took the keyboard) gives it back to the composer.
//
// Runs the REAL `cardKey`, `keyFocusOf`, `cardTakesFocus` and `cardDigits`
// from app/src/chat.ts, with only its two imports stubbed (React's store hook,
// and the ipc door, whose `windowOnHost` decides whether a `widens` choice is
// open), then reads ChatPane.tsx for the wiring a pure function cannot show:
// the listener is a capture-phase one, and Enter's send is the Send button's
// own function and predicate.
//
// Run from the repo root, as CI does. CHATKEYS_SRC / CHATKEYS_PANE point the
// check at other copies of chat.ts / ChatPane.tsx (to show it red).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const SRC = process.env.CHATKEYS_SRC ?? here("../src/chat.ts");
const PANE = process.env.CHATKEYS_PANE ?? here("../src/ChatPane.tsx");

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const expect = (what, got, want) => (eq(got, want) ? ok(what) : fail(`${what}: want ${JSON.stringify(want)}, got ${JSON.stringify(got)}`));

const src = fs.readFileSync(SRC, "utf8")
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = () => undefined;")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = async () => { throw new Error(\"no ipc here\"); }; const windowOnHost = () => globalThis.__onHost !== false;");
if (/^import /m.test(src)) fail("chat.ts gained an import this check does not stub — extend the stubs above");
const js = (await transformWithEsbuild(src, "chat.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

for (const name of ["cardKey", "keyFocusOf", "cardTakesFocus", "cardDigits"]) {
  if (typeof C[name] !== "function") fail(`chat.ts exports no ${name}`);
}
if (failed) {
  console.log(`\nchat-keys-check: ${failed} failure(s)`);
  process.exit(1);
}

// ── the prompts, shaped as the mock's (and serve's) screens ─────────────────
const prompt = (choices, extra = {}) => ({ waiting: true, readable: true, harness: "claude", session: "s", kind: "permission", fingerprint: "fp1", lines: [], choices, note: "", ...extra });
// Claude's Bash permission: Yes and No take a note; 2 widens what it may do.
const PERM = prompt([
  { n: 1, label: "Yes", highlighted: true, text: "amend" },
  { n: 2, label: "Yes, and always allow access to /x from this project", highlighted: false, host_only: "widens" },
  { n: 3, label: "No", highlighted: false, text: "amend" },
]);
// AskUserQuestion: three options, "Type something." (a field), "Chat about this".
const QUESTION = prompt([
  { n: 1, label: "Programming", highlighted: false },
  { n: 2, label: "Cats", highlighted: false },
  { n: 3, label: "Coffee", highlighted: false },
  { n: 4, label: "Type something.", highlighted: true, host_only: "text", text: "field" },
  { n: 5, label: "Chat about this", highlighted: false },
], { kind: "question" });
// A multi-select: three boxes, the free-text box, and "Chat about this" below.
const MULTI = prompt([
  { n: 1, label: "Cheese", highlighted: true, checked: false },
  { n: 2, label: "Olives", highlighted: false, checked: true },
  { n: 3, label: "Mushrooms", highlighted: false, checked: false },
  { n: 4, label: "Type something", highlighted: false, checked: false, host_only: "text", text: "field" },
  { n: 5, label: "Chat about this", highlighted: false },
], { kind: "question", multi: true });
// kimi's field: the reader cannot fill it, so the card draws it disabled.
const KIMI = prompt([
  { n: 1, label: "Approve", highlighted: true },
  { n: 2, label: "Reject with a reason", highlighted: false, host_only: "text" },
]);

const key = (k, mods = {}) => ({ key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, isComposing: false, repeat: false, ...mods });
const BASE = { focus: "none", sinceTook: null, escapeDepth: 0, shown: true, answering: false, selected: null, canSend: false };
const at = (over = {}) => ({ ...BASE, ...over });

// ── 1. a digit does what a click does ───────────────────────────────────────
expect("a digit selects a plain choice", C.cardKey(key("2"), at(), QUESTION), { act: "select", n: 2, choice: 2, field: false });
expect("a digit on a choice that takes a note selects it and focuses its note", C.cardKey(key("1"), at(), PERM), { act: "select", n: 1, choice: 1, field: true });
expect("a digit on a text field choice selects it and focuses its field", C.cardKey(key("4"), at(), QUESTION), { act: "select", n: 4, choice: 4, field: true });
expect("a digit on the selected choice unselects it, as a second click does", C.cardKey(key("2"), at({ selected: 2 }), QUESTION), { act: "select", n: 2, choice: null, field: false });
expect("a digit on another choice moves the selection", C.cardKey(key("3"), at({ selected: 1, focus: "card" }), PERM), { act: "select", n: 3, choice: 3, field: true });
expect("a digit ticks a multi-select's box", C.cardKey(key("1"), at(), MULTI), { act: "tick", n: 1 });
expect("a digit unticks a ticked box (the click's toggle)", C.cardKey(key("2"), at(), MULTI), { act: "tick", n: 2 });
expect("a digit ticks the free-text box, and does no more (no focus)", C.cardKey(key("4"), at(), MULTI), { act: "tick", n: 4 });
expect("a digit selects a multi-select's plain choice below the boxes", C.cardKey(key("5"), at(), MULTI), { act: "select", n: 5, choice: 5, field: false });
expect("a digit works with focus on another control (no native digit there)", C.cardKey(key("2"), at({ focus: "control" }), QUESTION), { act: "select", n: 2, choice: 2, field: false });
expect("the numpad's digit is the same key", C.cardKey(key("3"), at(), QUESTION), { act: "select", n: 3, choice: 3, field: false });

// ── 2. a digit with no choice is not the card's ─────────────────────────────
expect("a digit past the last choice is not the card's", C.cardKey(key("4"), at(), PERM), null);
expect("9 on a three-choice card is not the card's", C.cardKey(key("9"), at(), PERM), null);
expect("0 is never a choice", C.cardKey(key("0"), at(), QUESTION), null);
expect("a letter is not the card's", C.cardKey(key("y"), at(), PERM), null);
expect("Space is not the card's", C.cardKey(key(" "), at(), PERM), null);
expect("a choice the card draws disabled is not there for a key (kimi's field)", C.cardKey(key("2"), at(), KIMI), null);
globalThis.__onHost = false;
expect("a `widens` choice in a window driving a paired host is disabled, so not keyed", C.cardKey(key("2"), at(), PERM), null);
expect("…while its neighbours still are", C.cardKey(key("3"), at(), PERM), { act: "select", n: 3, choice: 3, field: true });
globalThis.__onHost = true;
expect("a `widens` choice on the console is keyed, as it is clickable", C.cardKey(key("2"), at(), PERM), { act: "select", n: 2, choice: 2, field: false });

// ── 3. a modifier, a text field, a terminal, a modal, an IME ────────────────
for (const [mod, name] of [["metaKey", "⌘ (App's ⌘1–9 nav chord)"], ["ctrlKey", "ctrl"], ["altKey", "⌥"], ["shiftKey", "shift"]]) {
  expect(`${name}+digit is not the card's`, C.cardKey(key("1", { [mod]: true }), at(), PERM), null);
  expect(`${name}+Enter is not the card's`, C.cardKey(key("Enter", { [mod]: true }), at({ canSend: true, selected: 1 }), PERM), null);
}
for (const focus of ["editable", "composer", "xterm"]) {
  expect(`a digit with focus in ${focus} is not the card's`, C.cardKey(key("1"), at({ focus }), PERM), null);
  expect(`Enter with focus in ${focus} is not the card's (its own Enter answers it)`, C.cardKey(key("Enter"), at({ focus, canSend: true, selected: 1 }), PERM), null);
}
expect("a digit under a modal (escape depth 1) is not the card's", C.cardKey(key("1"), at({ escapeDepth: 1 }), PERM), null);
expect("Enter under a modal is not the card's", C.cardKey(key("Enter"), at({ escapeDepth: 2, canSend: true, selected: 1 }), PERM), null);
expect("a digit while an IME composes is not the card's", C.cardKey(key("1", { isComposing: true }), at(), PERM), null);
expect("Enter while an IME composes is not the card's", C.cardKey(key("Enter", { isComposing: true }), at({ canSend: true, selected: 1 }), PERM), null);
expect("a held digit's repeats are not presses (a box would flicker)", C.cardKey(key("1", { repeat: true }), at(), MULTI), null);
expect("a card not on screen (under the reader, inert, hidden) takes no key", C.cardKey(key("1"), at({ shown: false }), PERM), null);

// ── 4. Enter is the Send button ─────────────────────────────────────────────
expect("Enter sends when the Send button is enabled (focus on nothing)", C.cardKey(key("Enter"), at({ canSend: true, selected: 1 }), PERM), { act: "send" });
expect("Enter sends from a focused choice, instead of clicking it again", C.cardKey(key("Enter"), at({ focus: "card", canSend: true, selected: 1 }), PERM), { act: "send" });
expect("Enter with nothing selected sends nothing, and is held from the focused choice", C.cardKey(key("Enter"), at({ focus: "card" }), PERM), { act: "hold" });
expect("Enter with nothing selected and focus on nothing sends nothing", C.cardKey(key("Enter"), at(), PERM), { act: "hold" });
expect("Enter with the Send button disabled (an empty field) sends nothing", C.cardKey(key("Enter"), at({ selected: 4, canSend: false }), QUESTION), { act: "hold" });
expect("Enter on another control is that control's (Cancel, a link, a nav row)", C.cardKey(key("Enter"), at({ focus: "control", canSend: true, selected: 1 }), PERM), null);
expect("Enter while answering does nothing (no second answer)", C.cardKey(key("Enter"), at({ answering: true, canSend: true, selected: 1 }), PERM), null);
expect("a digit while answering does nothing", C.cardKey(key("3"), at({ answering: true }), PERM), null);
expect("Enter sends a multi-select's ticks when its Send is enabled", C.cardKey(key("Enter"), at({ canSend: true }), MULTI), { act: "send" });

// ── 4b. typing at the card is typing a message ──────────────────────────────
// The card takes the keyboard from an EMPTY composer: a person who starts a
// message then must not answer the prompt with it.
const GRACE = C.CARD_GRACE_MS;
if (!(GRACE >= 500)) fail(`CARD_GRACE_MS is ${GRACE}: a key that soon after the card took the keyboard was aimed at the composer`);
const card = (over = {}) => at({ focus: "card", ...over });
expect("a letter at the card goes to the composer, with the keyboard", C.cardKey(key("t"), card(), PERM), { act: "composer" });
expect("a capital (shift) at the card goes to the composer", C.cardKey(key("T", { shiftKey: true }), card(), PERM), { act: "composer" });
expect("punctuation (shift+1 is !) at the card goes to the composer", C.cardKey(key("!", { shiftKey: true }), card(), PERM), { act: "composer" });
expect("…even while an answer is in flight (typing a message is always allowed)", C.cardKey(key("t"), card({ answering: true }), PERM), { act: "composer" });
expect("a digit just after the card took the keyboard goes to the composer", C.cardKey(key("2"), card({ sinceTook: 120 }), PERM), { act: "composer" });
expect("Enter just after the card took the keyboard goes to the composer", C.cardKey(key("Enter"), card({ sinceTook: 120, canSend: true, selected: 2 }), PERM), { act: "composer" });
expect("a digit once the grace is over is the card's again", C.cardKey(key("2"), card({ sinceTook: GRACE + 1 }), PERM), { act: "select", n: 2, choice: 2, field: false });
expect("a digit on a card the person focused (no take) is the card's at once", C.cardKey(key("2"), card({ sinceTook: null }), PERM), { act: "select", n: 2, choice: 2, field: false });
expect("Space at the card is not typing (it activates a focused choice)", C.cardKey(key(" "), card(), PERM), null);
expect("a digit with no choice at the card still does nothing", C.cardKey(key("0"), card(), PERM), null);
expect("a letter with focus on nothing is not the card's (nothing took the composer's keyboard)", C.cardKey(key("t"), at(), PERM), null);
expect("a letter on another control is that control's", C.cardKey(key("t"), at({ focus: "control" }), PERM), null);
expect("⌘+letter at the card is not typing (App's chords)", C.cardKey(key("k", { metaKey: true }), card(), PERM), null);
// The hijack, end to end: the card took the keyboard 3 s ago; "2 things⏎".
{
  const first = C.cardKey(key("2"), card({ sinceTook: 3000 }), PERM);
  const second = C.cardKey(key("t"), card({ sinceTook: 3000, selected: 2, canSend: true }), PERM);
  const enter = C.cardKey(key("Enter"), at({ focus: "composer", selected: 2, canSend: true }), PERM);
  if (first?.act !== "select" || second?.act !== "composer" || enter !== null) fail(`"2 things⏎" typed at the card must not send choice 2: got ${JSON.stringify([first, second, enter])}`);
  else ok(`"2 things⏎" selects 2, then hands the words AND the Enter to the composer, which cannot send while a prompt waits`);
}

// ── 5. where the keyboard is ────────────────────────────────────────────────
// Just enough of an Element for keyFocusOf: tag, classes, attributes, a parent.
function el(tag, { cls = [], attrs = {}, parent = null, editable = false } = {}) {
  const e = {
    tagName: tag.toUpperCase(), parent, isContentEditable: editable,
    classList: { contains: (c) => cls.includes(c) },
    getAttribute: (a) => (a in attrs ? String(attrs[a]) : null),
    hasAttribute: (a) => a in attrs,
    closest(sel) {
      const want = sel.replace(/^\./, "");
      for (let n = this; n; n = n.parent) if (n.classList.contains(want)) return n;
      return null;
    },
    contains(o) { for (let n = o; n; n = n.parent) if (n === this) return true; return false; },
  };
  return e;
}
const body = el("body");
const chat = el("div", { cls: ["chat"], parent: body });
const cardEl = el("div", { cls: ["chat-card", "chat-prompt"], attrs: { tabindex: -1, role: "group" }, parent: chat });
const choiceBtn = el("button", { cls: ["chat-choice"], parent: cardEl });
const boxLabel = el("label", { cls: ["chat-box"], parent: cardEl });
const boxInput = el("input", { attrs: { type: "checkbox" }, parent: boxLabel });
const freeBox = el("div", { cls: ["chat-box", "chat-box-free"], parent: cardEl });
const freeText = el("input", { cls: ["chat-answer-field"], attrs: { type: "text" }, parent: freeBox });
const noteField = el("input", { cls: ["chat-answer-field"], attrs: { type: "text" }, parent: cardEl });
const cancel = el("button", { cls: ["ctrl"], parent: cardEl });
const summary = el("summary", { parent: cardEl });
const composer = el("textarea", { cls: ["chat-field"], parent: chat });
const dock = el("div", { cls: ["dock"], parent: body });
const xtermRoot = el("div", { cls: ["xterm"], parent: dock });
const xtermTa = el("textarea", { cls: ["xterm-helper-textarea"], parent: xtermRoot });
const findField = el("input", { attrs: {}, parent: body });
const otherCardChoice = el("button", { cls: ["chat-choice"], parent: el("div", { parent: body }) });
for (const [what, e, want] of [
  ["nothing focused", null, "none"],
  ["the body", body, "none"],
  ["the card itself (where a new prompt puts the keyboard)", cardEl, "card"],
  ["one of the card's choices", choiceBtn, "card"],
  ["one of the card's boxes", boxInput, "card"],
  ["the free-text box's checkbox", el("input", { attrs: { type: "checkbox" }, parent: freeBox }), "card"],
  ["the free-text box's field", freeText, "editable"],
  ["a choice's note field", noteField, "editable"],
  ["the card's Cancel button", cancel, "control"],
  ["the card's folded rows (a summary)", summary, "control"],
  ["the chat's composer", composer, "composer"],
  ["the dock shell's xterm textarea", xtermTa, "xterm"],
  ["the find bar's field (an input with no type)", findField, "editable"],
  ["a select", el("select", { parent: body }), "editable"],
  ["a contenteditable", el("div", { editable: true, parent: body }), "editable"],
  ["a role=textbox", el("div", { attrs: { role: "textbox" }, parent: body }), "editable"],
  ["a link", el("a", { attrs: { href: "#" }, parent: body }), "control"],
  ["a tabindex=0 widget (the Files divider)", el("div", { attrs: { tabindex: 0, role: "separator" }, parent: body }), "control"],
  ["a choice of some OTHER card", otherCardChoice, "control"],
  ["a tabindex=-1 box outside the card", el("div", { attrs: { tabindex: -1 }, parent: body }), "none"],
]) expect(`focus on ${what} is "${want}"`, C.keyFocusOf(e, cardEl), want);

// ── 6. a new prompt takes the keyboard from an empty composer, once ─────────
const arrive = (over = {}) => C.cardTakesFocus({ fingerprint: "fp1", seen: null, focus: "composer", draft: "", escapeDepth: 0, shown: true, ...over });
expect("a new prompt takes the keyboard from an empty composer", arrive(), true);
expect("the same fingerprint again (a refresh) takes nothing a second time", arrive({ seen: "fp1" }), false);
expect("a NEW fingerprint after one already seen takes it again", arrive({ seen: "fp0" }), true);
expect("a composer holding text keeps the keyboard", arrive({ draft: "half a thought" }), false);
expect("a composer holding only a space keeps the keyboard (it is text)", arrive({ draft: " " }), false);
for (const focus of ["xterm", "editable", "card", "control", "none"]) {
  expect(`focus in ${focus} is never moved`, arrive({ focus }), false);
}
expect("an open sheet or menu keeps the keyboard", arrive({ escapeDepth: 1 }), false);
expect("a card not on screen takes nothing", arrive({ shown: false }), false);
expect("no readable prompt takes nothing", arrive({ fingerprint: null }), false);

// ── 7. the hint's digits ────────────────────────────────────────────────────
expect("the hint names the card's digits", C.cardDigits(PERM), "1–3");
expect("…every choice of a question", C.cardDigits(QUESTION), "1–5");
expect("…a multi-select's boxes and its choice below", C.cardDigits(MULTI), "1–5");
expect("…one digit for one open choice", C.cardDigits(KIMI), "1");
expect("…none when no choice is open", C.cardDigits(prompt([{ n: 1, label: "x", highlighted: true, host_only: "text" }])), null);

// ── 8. the wiring in ChatPane.tsx ───────────────────────────────────────────
{
  const pane = fs.readFileSync(PANE, "utf8");
  // Capture phase: decided before anything under the focus acts on the key
  // (a focused choice's native Enter), and before App's bubble-phase chords.
  if (!/addEventListener\("keydown", \w+, true\)/.test(pane) || !/cardKey\(/.test(pane)) fail("ChatPane registers no capture-phase keydown listener that asks cardKey");
  else ok("the card's keys are one capture-phase listener that asks cardKey");
  // A key handed back to the composer must go on to it: moved before, never prevented.
  const handBack = pane.indexOf('if (k.act === "composer")');
  const prevent = pane.indexOf("e.preventDefault();", pane.indexOf("cardKey(e,"));
  if (handBack < 0 || prevent < 0 || handBack > prevent) fail("the `composer` act is prevented before it moves the keyboard: the typed key would be lost");
  else ok("a key handed back to the composer moves the keyboard and is not prevented");
  // Enter's send is the Send button's: the same function, behind the same predicate.
  const pair = pane.match(/const send = selected && way \? \{ on: (\w+), run: (\w+) \} : p\.multi \? \{ on: (\w+), run: (\w+) \} : null;/);
  if (!pair) fail("ChatPane names no `send` pairing (selected ? answer : multi ? many) for Enter");
  else {
    const [, onA, runA, onM, runM] = pair;
    const btnA = new RegExp(`disabled=\\{!${onA}\\} onClick=\\{${runA}\\}[^>]*data-testid="chat-send-answer"`).test(pane);
    const btnM = new RegExp(`disabled=\\{!${onM}\\} onClick=\\{${runM}\\}[^>]*data-testid="chat-send-many"`).test(pane);
    if (!btnA || !btnM) fail(`Enter's send (${runA}/${runM} behind ${onA}/${onM}) is not what the Send buttons run and are enabled by`);
    else ok("Enter runs the Send button's own function, behind its own enabled state");
  }
  // Each site on its own: the choice button, and both kinds of box.
  const SHORTCUT = "aria-keyshortcuts={c.n <= 9 ? String(c.n) : undefined}";
  const button = pane.match(/<button className=\{"chat-choice"[^]*?data-testid=\{`chat-choice-\$\{c\.n\}`\}>/)?.[0] ?? "";
  const boxes = pane.match(/<input id=\{id\} type="checkbox"[^]*?data-testid=\{`chat-box-\$\{c\.n\}`\} \/>/g) ?? [];
  if (!button.includes(SHORTCUT)) fail("a choice's button does not name its digit in aria-keyshortcuts");
  else ok("a choice's button names its digit in aria-keyshortcuts");
  if (boxes.length !== 2 || !boxes.every((b) => b.includes(SHORTCUT))) fail(`a box does not name its digit in aria-keyshortcuts (${boxes.filter((b) => b.includes(SHORTCUT)).length} of ${boxes.length} boxes, want 2 of 2)`);
  else ok("a box and the free-text box name their digit in aria-keyshortcuts");
}

if (failed) {
  console.log(`\nchat-keys-check: ${failed} failure(s)`);
  process.exit(1);
}
console.log("\nchat-keys-check: all passed");
