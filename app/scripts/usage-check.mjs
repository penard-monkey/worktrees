// Guards the one rule the usage metrics rest on: nothing a person typed, and
// nothing that names their work, may become an event.
//
//   node app/scripts/usage-check.mjs        # exits non-zero on failure
//
// Two halves, both driven from the REAL source (the technique `race-check.mjs`
// and `zoom-check.mjs` use — slice the file, evaluate it, so this tests the
// edit and not a paraphrase of it):
//
//   1. `titleKey` / `comboName` / `keyForTarget` from usage.ts, exercised on
//      the titles this app actually carries — including a title that
//      interpolates a place name, which must produce NO key.
//   2. A static pass over the TSX: every control whose label or `data-testid`
//      is built from a value MUST carry an explicit `data-track`. That is belt
//      1, and it is the only one that covers a one-word slug — `titleKey`
//      cannot tell `messaging` from a label, and is not asked to.
//
// Invisible to every other suite: bats never loads the frontend, the unit tests
// never see a title, and the mock harness happily records whatever it is
// handed. The failure mode is silent by construction — a leak looks exactly
// like a working feature until someone opens the file.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const eq = (got, want, what) => { if (got !== want) fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`); };

// ── half 1: the resolver, from the real usage.ts ────────────────────────────
const SRC = read("../src/usage.ts");
// Everything from the key section down to the buffer, verbatim: the three
// exported pure functions and the constant they share. The import and the
// stateful half below it are left out (no Tauri here).
const from = SRC.indexOf("/** A key longer than this is not a name anyone wrote deliberately. */");
const to = SRC.indexOf("// ── the buffer ─");
if (from < 0 || to < 0 || to < from) {
  fail("usage.ts: the key-resolution section is gone — renamed markers?");
  console.error("\nusage-check: 1 failure(s)");
  process.exit(1);
}
const slice = SRC.slice(from, to).replace(/\bexport /g, "");
const js = (await transformWithEsbuild(slice, "usage-slice.ts", { loader: "ts" })).code;
const { titleKey, comboName, keyForTarget } = new Function(`${js}; return { titleKey, comboName, keyForTarget };`)();

// The three the brief names, straight off real controls in App.tsx.
eq(titleKey("Places — pinned; click to unpin (⌘B)"), "places", "titleKey/places");
eq(titleKey("attention — dirty / …"), "attention", "titleKey/attention");
eq(titleKey("Enter — open the session"), "enter", "titleKey/enter");
// …and the rest of the shapes the app really uses
eq(titleKey("Layout: auto — click to cycle (auto → stacked → side by side)"), "layout", "titleKey/layout");
eq(titleKey("new worktree"), "new-worktree", "titleKey/new-worktree");
eq(titleKey("Previous match (⇧⏎)"), "previous-match", "titleKey/previous-match");
eq(titleKey("settings — update available"), "settings", "titleKey/settings");

// ⚠ THE ASSERTION. A row's title is `"<the name you gave it> — <slug>"`, and a
// slug is hyphenated by construction. It must produce nothing at all.
eq(titleKey("standup-and-daily-work — random-work"), null, "titleKey refuses a slug");
eq(titleKey("ui-tweaks"), null, "titleKey refuses a bare slug");
eq(titleKey("/Users/davidpena/workspace/worktrees"), null, "titleKey refuses a path");
eq(titleKey("feat/redesign"), null, "titleKey refuses a branch");
eq(titleKey("café ristretto"), null, "titleKey refuses non-ASCII");
eq(titleKey("x".repeat(60)), null, "titleKey refuses an over-long key");
eq(titleKey(""), null, "titleKey refuses nothing");
eq(titleKey(null), null, "titleKey refuses null");

// Chords are built from `e.code`, never `e.key` — see the comment there.
const K = (o) => ({ metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, code: "", key: "", ...o });
eq(comboName(K({ metaKey: true, code: "KeyB", key: "b" })), "cmd-b", "comboName/cmd-b");
eq(comboName(K({ metaKey: true, shiftKey: true, code: "KeyT", key: "T" })), "cmd-shift-t", "comboName/cmd-shift-t");
eq(comboName(K({ metaKey: true, code: "Digit1", key: "1" })), "cmd-1", "comboName/cmd-1");
eq(comboName(K({ metaKey: true, code: "Equal", key: "=" })), "cmd-equal", "comboName/cmd-equal");
// ⌥ composes with the layout, so `e.key` is "≠" here and would name nothing
eq(comboName(K({ metaKey: true, altKey: true, code: "Equal", key: "≠" })), "cmd-alt-equal", "comboName/⌥ variant");
// no modifier = a keystroke, and a keystroke is TEXT
eq(comboName(K({ code: "KeyA", key: "a" })), null, "comboName refuses a bare key");
eq(comboName(K({ code: "Escape", key: "Escape" })), null, "comboName refuses Escape");
eq(comboName(K({ metaKey: true, code: "Escape", key: "Escape" })), null, "comboName refuses ⌘Escape");

// keyForTarget's precedence, against a tiny stub of the DOM contract it uses.
function el({ track, testid, title, tag = "button", parent = null }) {
  const self = {
    dataset: track ? { track } : {},
    getAttribute: (n) => (n === "data-testid" ? testid ?? null : n === "title" ? title ?? null : null),
    tag,
    parent,
  };
  self.closest = (sel) => {
    for (let n = self; n; n = n.parent) {
      if (sel === "[data-track]" && n.dataset.track) return n;
      if (sel === "[data-testid]" && n.getAttribute("data-testid")) return n;
      if (sel === "button, [role=button]" && n.tag === "button") return n;
    }
    return null;
  };
  return self;
}
eq(keyForTarget(el({ track: "nav.row", testid: "row", title: "my-project — my-slug" })), "nav.row", "data-track wins");
eq(keyForTarget(el({ testid: "attn-filter", title: "attention — dirty" })), "attn-filter", "data-testid beats title");
eq(keyForTarget(el({ testid: "sync-mini|/Users/me/workspace/thing" })), null, "an interpolated data-testid is not a key");
eq(keyForTarget(el({ title: "Places — pinned; click to unpin (⌘B)" })), "places", "title, on a button");
eq(keyForTarget(el({ title: "Places — pinned", tag: "span" })), null, "a title on a non-control is not a key");
eq(keyForTarget(null), null, "no target, no key");

// ── half 2: the controls that MUST carry an explicit key ────────────────────
// Each entry is a line-matching probe over the real TSX: a fragment that pins
// the site (a title or testid built from a value), and the `data-track` that
// site must carry within a few lines of it. `titleKey` cannot save these — a
// one-word slug reads exactly like a label — so this is the assertion that the
// belt is actually fastened.
const SITES = [
  ["../src/App.tsx", "title={p.declared?.title ? `${p.declared.title} — ${p.slug}` : p.slug}", "nav.row", "the place row's title is the place's name"],
  ["../src/App.tsx", "data-testid={`stray-mini|${pv.root}`}", "nav.project.strays", "this data-testid carries the project root"],
  ["../src/App.tsx", "data-testid={`sync-mini|${pv.root}`}", "nav.project.sync", "this data-testid carries the project root"],
  ["../src/App.tsx", 'className="project-h"', "nav.project", "the project header's label is the repo path"],
  ["../src/App.tsx", "title={`config unreadable", "nav.project.broken", "this title carries the config parse error"],
  ["../src/App.tsx", "title={`${closeSess} was adopted", "place.close.confirm", "this title STARTS with the tmux session name"],
  ["../src/App.tsx", 'className={"qs-row"', "switch.row", "a ⌘K row is a place"],
  ["../src/App.tsx", 'className="resume-row"', "home.resume", "a Home resume row is a place"],
  ["../src/FilesPane.tsx", "title={title}", "files.row", "a tree row's title is the file path"],
  ["../src/ProfilesPanel.tsx", "title={p.dir ?? undefined}", "profiles.pick", "this title is the profile's config directory"],
];
for (const [file, needle, track, why] of SITES) {
  const lines = read(file).split("\n");
  const i = lines.findIndex((l) => l.includes(needle));
  if (i < 0) {
    fail(`${file}: cannot find \`${needle}\` — moved or rewritten; re-check that ${why}, then update this probe`);
    continue;
  }
  const near = lines.slice(Math.max(0, i - 8), i + 8).join("\n");
  if (!near.includes(`data-track="${track}"`)) {
    fail(`${file}:${i + 1}: ${why}, and the control has no \`data-track="${track}"\` — its key would be built from user text`);
  }
}

// ── half 3: a DYNAMIC title may never be a control's ONLY key ───────────────
// Half 2 pins the controls we know about. This one covers the ones nobody has
// written yet, and it exists because `titleKey`'s refusals have a hole in the
// middle of them: they catch slugs, paths, branches and non-ASCII, but NOT a
// name with spaces in it. `"Quokka Fanclub HQ — messaging"` reduces to
// `quokka-fanclub-hq` — a perfectly well-formed key made entirely of someone's
// typing. The place row survives that only because it carries `data-track`.
//
// So the rule here is STRUCTURAL rather than lexical, because a lexical rule
// cannot be written: if a control's `title` is an expression, its key may not
// be derived from that title, whatever the expression happens to evaluate to
// today. It needs an explicit `data-track`, or a `data-testid` that is a
// literal — a constant testid is source, and `keyForTarget` prefers it to the
// title, so the title never gets read.
//
// Every button this flagged when it was written had literal branches and was
// therefore harmless AT THAT MOMENT; the point is that nothing was stopping the
// next edit from interpolating a value into any of them, silently.

/** Opening JSX tags, with their attribute text. A hand-rolled walk rather than
 *  a parser: braces nest, and template literals contain both braces and quotes
 *  (`title={`${n} worktree${n === 1 ? "" : "s"} outside…`}`), so a regex over
 *  `<button[^>]*>` stops at the first `>` inside an expression and reads half
 *  an element. Strings are opaque; brace depth decides where the tag ends. */
function openingTags(src) {
  const out = [];
  for (let i = 0; i < src.length; i++) {
    if (src[i] !== "<" || !/[A-Za-z]/.test(src[i + 1] ?? "")) continue;
    let j = i + 1;
    while (j < src.length && /[A-Za-z0-9_.]/.test(src[j])) j++;
    const name = src.slice(i + 1, j);
    let depth = 0, quote = null, k = j;
    for (; k < src.length; k++) {
      const c = src[k];
      if (quote) { if (c === "\\") k++; else if (c === quote) quote = null; continue; }
      if (c === '"' || c === "'" || c === "`") { quote = c; continue; }
      if (c === "{") depth++;
      else if (c === "}") depth--;
      else if (c === ">" && depth === 0) break;
      // a bare `<` at depth 0 means the previous match was not a tag at all
      // (a comparison, a generic) — abandon it rather than swallow the file
      else if (c === "<" && depth === 0) { k = -1; break; }
    }
    if (k < 0 || k >= src.length) continue;
    out.push({ name, attrs: src.slice(j, k), line: src.slice(0, i).split("\n").length });
  }
  return out;
}

/** One attribute's value, and whether it is a plain string literal.
 *  `title="x"` and `title={"x"}` are literal; `title={expr}` is not. */
function jsxAttr(attrs, want) {
  const m = attrs.match(new RegExp(`(?<![\\w-])${want}=`));
  if (!m) return null;
  let k = m.index + m[0].length;
  if (attrs[k] === '"' || attrs[k] === "'") {
    const q = attrs[k];
    return { raw: attrs.slice(k + 1, attrs.indexOf(q, k + 1)), literal: true };
  }
  if (attrs[k] !== "{") return null;
  let d = 0, quote = null;
  const start = k;
  for (; k < attrs.length; k++) {
    const c = attrs[k];
    if (quote) { if (c === "\\") k++; else if (c === quote) quote = null; continue; }
    if (c === '"' || c === "'" || c === "`") { quote = c; continue; }
    if (c === "{") d++;
    else if (c === "}") { d--; if (!d) break; }
  }
  const raw = attrs.slice(start + 1, k);
  return { raw, literal: /^\s*(["'])(?:(?!\1).)*\1\s*$/.test(raw) };
}

const TSX = fs
  .readdirSync(fileURLToPath(new URL("../src", import.meta.url)))
  .filter((f) => f.endsWith(".tsx"))
  .sort();
const offenders = [];
for (const f of TSX) {
  const src = read(`../src/${f}`);
  for (const t of openingTags(src)) {
    // only things a click can key off a title: `keyForTarget` reads a title
    // from `button, [role=button]` and from nothing else, so an <input> or a
    // <span> with an interpolated title is not a hole.
    const isButton =
      t.name === "button" ||
      /(?<![\w-])role=(["'])button\1/.test(t.attrs) ||
      /(?<![\w-])role=\{\s*(["'])button\1\s*\}/.test(t.attrs);
    if (!isButton) continue;
    const title = jsxAttr(t.attrs, "title");
    if (!title || title.literal) continue; // a fixed title is titleKey's job
    const track = jsxAttr(t.attrs, "data-track");
    const testid = jsxAttr(t.attrs, "data-testid");
    // an interpolated data-track/testid is not a constant either
    if ((track && !track.raw.includes("${")) || (testid && !testid.raw.includes("${"))) continue;
    offenders.push(`${f}:${t.line}  title={${title.raw.replace(/\s+/g, " ").trim().slice(0, 64)}}`);
  }
}
if (!TSX.length) fail("no .tsx files found — did the scanner's path move?");
if (offenders.length) {
  fail(
    `${offenders.length} button(s) whose title is an EXPRESSION and whose only key would be built from it.\n` +
      offenders.map((o) => `        ${o}`).join("\n") +
      `\n      Give each an explicit data-track="…" (or a constant data-testid).` +
      `\n      titleKey cannot save these: it refuses slugs and paths, but a name` +
      `\n      with spaces — "Quokka Fanclub HQ" — reduces to a valid-looking key.`,
  );
}

// ── half 4: a terminal keystroke is not one of our chords ───────────────────
// xterm calls preventDefault on Ctrl+C, Ctrl+R and the tmux prefix and hands
// them to the pty, so they reach `trackChord` looking exactly like an app chord
// that did something. Recording them leaks nothing — a chord name is a physical
// key, not a character — but `chord.ctrl-c` outranks every real control within a
// day, and drowns the one question this feature exists to answer.
const tcFrom = SRC.indexOf("/** The chord that just fired");
const tcTo = SRC.indexOf("/** Install the listeners");
if (tcFrom < 0 || tcTo < 0 || tcTo < tcFrom) {
  fail("usage.ts: `trackChord` is gone — renamed markers?");
} else {
  const tcJs = (await transformWithEsbuild(SRC.slice(tcFrom, tcTo).replace(/\bexport /g, ""), "tc.ts", { loader: "ts" })).code;
  // `e.target instanceof Element` needs an Element to be an instance OF.
  class Elem {
    constructor(inTerm) { this.inTerm = inTerm; }
    closest(sel) { return sel === ".term-host" && this.inTerm ? this : null; }
  }
  const got = [];
  const trackChord = new Function("track", "comboName", "Element", `${tcJs}; return trackChord;`)(
    (k) => got.push(k), comboName, Elem);
  const key = (o) => ({ defaultPrevented: true, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, code: "", ...o });
  const fired = (e) => { got.length = 0; trackChord(e); return got[0] ?? null; };

  const inTerm = new Elem(true), outside = new Elem(false);
  eq(fired(key({ ctrlKey: true, code: "KeyC", target: inTerm })), null, "Ctrl+C in the terminal is the pty's");
  eq(fired(key({ ctrlKey: true, code: "KeyB", target: inTerm })), null, "the tmux prefix is the pty's");
  eq(fired(key({ ctrlKey: true, shiftKey: true, code: "KeyR", target: inTerm })), null, "…shift does not make it ours");
  // ⌘ never reaches the pty, so it is ours wherever it is pressed — and the
  // terminal is where most of the app's chords ARE pressed.
  eq(fired(key({ metaKey: true, code: "KeyB", target: inTerm })), "chord.cmd-b", "⌘B with the terminal focused is still ours");
  eq(fired(key({ metaKey: true, ctrlKey: true, code: "KeyK", target: inTerm })), "chord.cmd-ctrl-k", "⌘ wins over the ctrl rule");
  // outside the terminal a ctrl chord is an app chord like any other
  eq(fired(key({ ctrlKey: true, code: "KeyC", target: outside })), "chord.ctrl-c", "Ctrl+C elsewhere is ours");
  eq(fired(key({ ctrlKey: true, code: "KeyC", target: null })), "chord.ctrl-c", "no target, no terminal");
  // and the gate that was already there
  eq(fired({ ...key({ metaKey: true, code: "KeyB", target: outside }), defaultPrevented: false }), null, "a chord that did nothing is not recorded");
}

// The Rust side refuses the same shapes, and it is the last belt: if its
// allowlist ever becomes a denylist, a slug walks straight into the file.
const RS = read("../src-tauri/src/lib.rs");
if (!/fn valid_token\(s: &str\) -> bool \{[\s\S]*?is_ascii_alphanumeric\(\)[\s\S]*?\}/.test(RS)) {
  fail("lib.rs: `valid_token` no longer allowlists ASCII identifier characters — the backend would store whatever it is sent");
}
if (!/fn valid_event\(/.test(RS)) fail("lib.rs: `valid_event` is gone — nothing checks a batch on the way in");

if (bad) { console.error(`\nusage-check: ${bad} failure(s)`); process.exit(1); }
console.log(`usage-check: ok — key resolution, chord names, the terminal-keystroke rule, ${SITES.length} pinned controls, and ${TSX.length} .tsx files with no unkeyed dynamic-title button`);
