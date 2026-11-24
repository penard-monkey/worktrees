// The macOS menu bar against the handlers its items replay into.
//
//   node app/scripts/menu-check.mjs        # from the repo root; exits non-zero on failure
//
// An item in winmenu.rs carries a CHORD, and choosing it dispatches that chord
// as a keydown in the focused window (app/src/menu.ts). Nothing compiles the
// two together: a chord renamed in App.tsx, or an item added with the wrong
// key, is a menu item that does nothing. So this parses `ITEMS` out of the Rust
// and drives each chord through the REAL handler, sliced from App.tsx. It also
// runs the real menu.ts: one press is one act.
//
// Overrides, for mutations: MENU_RS, MENU_APP, MENU_TS
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const read = (p) => fs.readFileSync(p, "utf8");
let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const eq = (got, want, what) => { if (JSON.stringify(got) !== JSON.stringify(want)) fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`); };

const rs = read(process.env.MENU_RS ?? here("../src-tauri/src/winmenu.rs")).split(/\n#\[cfg\(test\)\]\s*\nmod tests\b/)[0];
const app = read(process.env.MENU_APP ?? here("../src/App.tsx"));
const menuTs = read(process.env.MENU_TS ?? here("../src/menu.ts"));
const companion = read(here("../companion/Companion.tsx"));

// ── the items ───────────────────────────────────────────────────────────────
const table = rs.slice(rs.indexOf("const ITEMS"), rs.indexOf("\n];", rs.indexOf("const ITEMS")));
const items = [];
for (const l of table.split("\n")) {
  const c = /^\s*chord\("([^"]+)", "([^"]+)", "([^"]+)", "([^"]+)", "([^"]+)"\)(.*),$/.exec(l);
  const a = /^\s*act\("([^"]+)", "([^"]+)"\)(.*),$/.exec(l);
  const tail = c?.[6] ?? a?.[3] ?? "";
  const scope = /\.scope\(Scope::(\w+)\)/.exec(tail)?.[1] ?? "Main";
  const withM = /\.with\((true|false), (true|false)\)/.exec(tail);
  if (c) items.push({ id: c[1], scope, key: c[4], code: c[5], shift: withM?.[1] === "true", alt: withM?.[2] === "true", focus: /\.at_focus\(\)/.test(tail) });
  else if (a) items.push({ id: a[1], scope, act: true });
  else if (/\b(chord|act)\(/.test(l)) fail(`winmenu.rs: an item this check cannot read: ${l.trim()}`);
}
if (items.length < 20) fail(`winmenu.rs: only ${items.length} items parsed — is ITEMS where this looks?`);

// ── every chord reaches the page's own handler ──────────────────────────────
const js = async (src) => (await transformWithEsbuild(src, "x.ts", { loader: "ts" })).code;
const hFrom = app.indexOf("    const onKey = (e: KeyboardEvent) => {\n      // Esc closes the find bar");
const hTo = app.indexOf('    window.addEventListener("keydown", onKey);', hFrom);
const nav = app.match(/\nconst isTextField = [\s\S]*?\nconst navChordDir = [\s\S]*?\n};\n/);
const navChords = app.match(/const NAV_CHORDS[^\n]*\n/);
if (hFrom < 0 || hTo < 0 || !nav || !navChords) { fail("App.tsx: the keydown handler's markers are gone"); process.exit(1); }
const handler = await js(nav[0] + navChords[0] + read(here("../src/zoomkeys.ts")).replace(/^export /gm, "") + app.slice(hFrom, hTo));
globalThis.Element ??= class {};
/** Press `i` with the handler's free names recorded; what it called. */
function drive(i, modal) {
  const calls = [];
  const real = {
    modalOpen: () => modal, onlySettingsOpen: () => false, DEFAULTS: { app_zoom: 1, files_md_zoom: 100 },
    clampZoom: (v) => v, stepZoom: (v, d) => v + d, clampMdZoom: (v) => v, stepMdZoom: (v, d) => v + d,
    findTarget: () => "file", lastSurface: { current: null }, navState: { current: {} }, document: { querySelector: () => null },
    navStepRef: { current: () => calls.push("navStep") }, newWorktreeRef: { current: () => calls.push("newWorktree") },
    invoke: (c) => { calls.push(c); return Promise.resolve(); },
    // Off defaults, so a reset has something to reset.
    keyRef: { current: { mdPreview: true, mdZoom: 125, appZoom: 1.5, ideasOn: true, selectedPath: "/p", filesTabOpen: true, dockFile: "a.md", dockShown: true, dockTab: "terminal" } },
  };
  const env = new Proxy(real, {
    has: (t, k) => typeof k === "string" && (k in t || !(k in globalThis)),
    get: (t, k) => (k === Symbol.unscopables ? undefined : k in t ? t[k] : (...a) => { calls.push(k); return a; }),
  });
  const onKey = new Function("env", `with (env) { ${handler.replace(/^"use strict";/, "")}; return onKey; }`)(env);
  let prevented = false;
  onKey({ key: i.key, code: i.code, metaKey: true, ctrlKey: false, altKey: i.alt, shiftKey: i.shift, repeat: false, target: {}, preventDefault: () => { prevented = true; } });
  return { calls, prevented };
}
for (const i of items.filter((x) => !x.act && x.scope !== "Companion")) {
  const { calls, prevented } = drive(i, false);
  if (i.focus) {
    // The focused element's: the window's handler must not act on it as well.
    eq(calls, [], `${i.id}: handled by the focused element, so the window's handler leaves it`);
    continue;
  }
  if (!calls.length || !prevented) fail(`${i.id}: ⌘${i.shift ? "⇧" : ""}${i.alt ? "⌥" : ""}${i.key} does nothing in App.tsx's handler (called ${JSON.stringify(calls)}, prevented ${prevented})`);
  const behind = drive(i, true);
  if (i.id.startsWith("zoom-")) { if (!behind.calls.length) fail(`${i.id}: page zoom works behind a dialog`); }
  else eq(behind.calls, [], `${i.id}: does nothing behind a dialog`);
}
// The elements that own the focus items.
if (!/k === "s"\) \{\s*e\.preventDefault\(\);\s*e\.stopPropagation\(\);\s*onSave\(\);/.test(read(here("../src/FilesPane.tsx")))) fail("FilesPane.tsx: ⌘S no longer saves, so File → Save does nothing");
if (!/k === "g"\)\) \{[\s\S]{0,120}e\.shiftKey\) onPrev\(\); else onNext\(\)/.test(read(here("../src/Find.tsx")))) fail("Find.tsx: ⌘G / ⌘⇧G no longer step, so Edit → Find Next does nothing");
// Every chordless item has a hand in its window.
for (const i of items.filter((x) => x.act)) {
  const src = i.scope === "Companion" ? companion : app;
  if (!src.includes(`id === "${i.id}"`)) fail(`${i.id}: no window acts on it`);
}
if (!/useEffect\(\(\) => wireMenu\("main", /.test(app)) fail("App.tsx does not listen to the menu bar");
if (!/useEffect\(\(\) => wireMenu\("companion", /.test(companion)) fail("Companion.tsx does not listen to the menu bar");
if (!/if \(modalOpen\(\)\) return;\s*if \(id === "add-project"\)/.test(app)) fail("App.tsx: a chordless item acts behind a dialog");

// ── one press, one act: the real menu.ts ────────────────────────────────────
{
  let heard = null, label = null, keydown = null, now = 1000;
  const sent = [];
  const node = (name) => ({ dispatchEvent: (e) => sent.push([name, e]) });
  const body = node("body"), field = node("field");
  globalThis.window = { addEventListener: (_n, f, capture) => { keydown = f; if (capture !== true) fail("menu.ts must watch keydown in the capture phase"); }, removeEventListener: () => {} };
  globalThis.document = { body, activeElement: field };
  globalThis.KeyboardEvent = class { constructor(t, o) { Object.assign(this, { type: t }, o); } };
  Object.defineProperty(globalThis, "performance", { value: { now: () => now }, configurable: true });
  globalThis.__listen = (l, h) => { label = l; heard = h; return Promise.resolve(() => {}); };
  if (!/import \{ listenMenu, type MenuChosen \} from "\.\/ipc";/.test(menuTs)) fail("menu.ts's import changed — the inlining is stale");
  const code = (await transformWithEsbuild(menuTs.replace(/^import .*$/m, "const listenMenu = (l: string, h: any) => (globalThis as any).__listen(l, h);\ntype MenuChosen = any;"), "menu.ts", { loader: "ts", format: "esm" })).code;
  const M = await import("data:text/javascript;base64," + Buffer.from(code).toString("base64"));
  const acts = [];
  M.wireMenu("companion", (id) => acts.push(id));
  eq(label, "companion", "a window listens as itself");
  const zoomIn = { id: "zoom-in", chord: { key: "+", code: "Equal", shift: false, alt: false, focus: false } };
  const press = (o) => keydown({ isTrusted: true, metaKey: true, altKey: false, ...o });

  heard(zoomIn);
  eq(sent.map(([n, e]) => [n, e.type, e.key, e.code, e.metaKey, e.bubbles, e.cancelable]), [["body", "keydown", "+", "Equal", true, true, true]], "a clicked item is one keydown at the window");
  sent.length = 0;
  press({ key: "+", code: "Equal" });
  now += 20; heard(zoomIn);
  eq(sent.length, 0, "a press the page already saw is not run again by the menu");
  now += M.SAME_PRESS_MS; heard(zoomIn);
  eq(sent.length, 1, "a later click of the same item is an act of its own");
  sent.length = 0;
  press({ key: "b", code: "KeyB", repeat: true });
  heard({ id: "toggle-places", chord: { key: "b", code: "KeyB", shift: false, alt: false, focus: false } });
  eq(sent.length, 0, "a held ⌘B's repeats do not toggle through the menu");
  heard({ id: "toggle-dock", chord: { key: "j", code: "KeyJ", shift: false, alt: false, focus: false } });
  eq(sent.length, 1, "another item is not that press");
  sent.length = 0; now += 5000;
  keydown({ isTrusted: false, metaKey: true, altKey: false, key: "+", code: "Equal" });
  heard(zoomIn); heard(zoomIn);
  eq(sent.length, 2, "two quick clicks are two steps: the replay itself is not a press");
  sent.length = 0;
  heard({ id: "save", chord: { key: "s", code: "KeyS", shift: false, alt: false, focus: true } });
  eq(sent.map(([n]) => n), ["field"], "an element's item goes to the focused element");
  heard({ id: "new-idea", chord: null });
  eq(acts, ["new-idea"], "an item with no chord is its window's act");
}
// `listen` with no target hears what was sent to the OTHER window.
if (!/tauriListen<MenuChosen>\("menu:item", [^\n]*\{ target: \{ kind: "WebviewWindow", label \} \}\)/.test(read(here("../src/ipc.ts")))) fail("ipc.ts: listenMenu is not targeted at its own window — both windows would act");
if (!/const EVENT: &str = "menu:item";/.test(rs) || !/emit_to\(EventTarget::webview_window\(&to\), EVENT,/.test(rs)) fail("winmenu.rs: the event is not sent to one window");

if (bad) { console.error(`\nmenu-check: ${bad} failure(s)`); process.exit(1); }
console.log(`menu-check: ${items.length} items, ${items.filter((i) => !i.act).length} chords driven through the real handler, one press one act`);
