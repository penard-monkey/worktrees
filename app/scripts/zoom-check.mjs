// Guards the app-zoom step table against the Rust clamp it is fed into.
//
//   node app/scripts/zoom-check.mjs        # exits non-zero on failure
//
// `ZOOM_STEPS` (settings.ts) is what ⌘+/⌘− walks and what the Settings slider
// renders; `set_zoom` (lib.rs) re-clamps whatever arrives, because an IPC
// boundary must not trust its caller — a NaN or a wild factor there is a window
// nobody can zoom back out of. The two are therefore a MIRROR, and mirrors
// drift: add a 4× step and the slider offers it, the label says 400%, the
// setting persists as 4 — and the window silently stops at 300%, because the
// Rust clamped it on the way through. Nothing reads the applied zoom back, so
// there is no assertion anywhere that could notice.
//
// Same family as `dnd-check.mjs`: a static check standing in for one the bats
// suite, the unit tests and the mock harness structurally cannot make.
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };

// ── the TS side: ZOOM_STEPS ────────────────────────────────────────────────
const ts = read("../src/settings.ts");
const stepsSrc = ts.match(/export const ZOOM_STEPS = \[([^\]]*)\]/);
if (!stepsSrc) fail("settings.ts: no `export const ZOOM_STEPS = [...]` — renamed or removed?");
const steps = stepsSrc ? stepsSrc[1].split(",").map((n) => Number(n.trim())).filter((n) => !Number.isNaN(n)) : [];
if (steps.length < 2) fail(`settings.ts: ZOOM_STEPS parsed as ${JSON.stringify(steps)} — expected at least two numbers`);

// ── the Rust side: the clamp inside set_zoom ───────────────────────────────
const rs = read("../src-tauri/src/lib.rs");
const body = rs.match(/async fn set_zoom\([\s\S]*?\n\}/);
if (!body) fail("lib.rs: no `async fn set_zoom` — renamed or removed?");
const clamp = body && body[0].match(/\.clamp\(\s*([0-9.]+)\s*,\s*([0-9.]+)\s*\)/);
if (body && !clamp) fail("lib.rs: set_zoom no longer clamps its factor — an IPC caller can now hand the webview anything");

// ── they must agree ────────────────────────────────────────────────────────
if (steps.length && clamp) {
  const [lo, hi] = [Number(clamp[1]), Number(clamp[2])];
  for (const s of steps) {
    if (s < lo || s > hi) fail(`ZOOM_STEPS has ${s}, outside the Rust clamp ${lo}..${hi} — the slider would offer a size the window silently refuses`);
  }
  // 1 must be reachable, or ⌘0 lands on a step the table does not contain and
  // the next ⌘+ steps from a value `stepZoom` cannot find (index -1 → 0).
  if (!steps.includes(1)) fail("ZOOM_STEPS has no 1 — ⌘0 resets to a value that is not a step");
  const sorted = [...steps].every((s, i) => i === 0 || s > steps[i - 1]);
  if (!sorted) fail(`ZOOM_STEPS is not strictly ascending (${JSON.stringify(steps)}) — stepZoom walks it by index`);
}

if (bad) { console.error(`\nzoom-check: ${bad} failure(s)`); process.exit(1); }
console.log(`zoom-check: table ok — ${steps.length} steps (${steps[0]}..${steps[steps.length - 1]}), Rust clamp ${clamp[1]}..${clamp[2]}`);

// ── the chord itself, driven from the REAL source ──────────────────────────
// Same technique as `race-check.mjs`: slice the handler out of App.tsx between
// stable markers and evaluate it with stubs, so this tests the edit rather than
// a paraphrase of it. What no other suite can see: ⌘ vs ⌘⌥ routing, the
// keyRef mutation that makes a fast double-tap step twice, and the deliberate
// decision to let ⌘+ through while a sheet is open.
import { transformWithEsbuild } from "vite";

const APP = fileURLToPath(new URL("../src/App.tsx", import.meta.url));
const appLines = fs.readFileSync(APP, "utf8").split("\n");
const from = appLines.findIndex((l) => l.includes("const dir = e.metaKey && !e.ctrlKey ? zoomDir(e) : undefined;"));
const to = appLines.findIndex((l, i) => i > from && l.includes("if (!(e.metaKey || e.ctrlKey) || e.repeat"));
// The direction tables + `zoomDir` live at module scope; take them verbatim too,
// so a layout face added there is a face this check actually exercises.
const dirTables = fs.readFileSync(APP, "utf8").match(/const ZOOM_BY_KEY[\s\S]*?\nconst zoomDir = [^\n]*\n/);
if (from < 0 || to < 0 || !dirTables) {
  fail("App.tsx: the ⌘/⌘⌥ zoom block's markers are gone — the chord is unchecked");
  console.error("\nzoom-check: 1 failure(s)");
  process.exit(1);
}
const BLOCK = appLines.slice(from, to).join("\n");

// settings.ts imports the Tauri bridge; stub it so the module loads under node.
const settingsSrc = ts.replace(/^import \{ invoke \}.*$/m, "const invoke = () => Promise.resolve();");
const load = async (src, name) => {
  const js = (await transformWithEsbuild(src, name, { loader: "ts", format: "esm" })).code;
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
};
const S = await load(settingsSrc, "settings.ts");
const handlerSrc = `
${dirTables[0]}
export function build(env: any) {
  const { keyRef, updatePanels, updateSettings, clampMdZoom, stepMdZoom, clampZoom, stepZoom, DEFAULTS } = env;
  return (e: any) => {
${BLOCK}
    return "fellthrough";
  };
}`;
const { build } = await load(handlerSrc, "zoom-chord.ts");

const keyRef = { current: {} };
let panels = [], sets = [];
const onKey = build({
  keyRef, DEFAULTS: S.DEFAULTS,
  clampMdZoom: S.clampMdZoom, stepMdZoom: S.stepMdZoom, clampZoom: S.clampZoom, stepZoom: S.stepZoom,
  updatePanels: (p) => panels.push(p), updateSettings: (p) => sets.push(p),
});

/** Press `key` with the given modifiers against `state`; report what happened. */
function press(key, { alt = false, meta = true, ctrl = false, code = "" } = {}, state = {}) {
  panels = []; sets = [];
  keyRef.current = { mdPreview: false, mdZoom: 100, appZoom: 1, switchOpen: false, settingsOpen: false, ...state };
  let prevented = false;
  const through = onKey({ key, code, metaKey: meta, ctrlKey: ctrl, altKey: alt, preventDefault: () => { prevented = true; } });
  return { prevented, sets, panels, through: through === "fellthrough", kr: keyRef.current };
}
const eq = (got, want, what) => { if (JSON.stringify(got) !== JSON.stringify(want)) fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`); };

// ⌘ alone is the APP's size, on every face of every key.
for (const k of ["+", "="]) eq(press(k).sets, [{ app_zoom: 1.1 }], `⌘${k} steps the app up`);
for (const k of ["-", "_"]) eq(press(k).sets, [{ app_zoom: 0.9 }], `⌘${k} steps the app down`);
eq(press("0", {}, { appZoom: 1.75 }).sets, [{ app_zoom: 1 }], "⌘0 resets the app to 1");
eq(press("+", {}, { appZoom: 3 }).sets, [], "⌘+ at the ceiling writes nothing");
eq(press("-", {}, { appZoom: 0.8 }).sets, [], "⌘− at the floor writes nothing");

// The keyRef mutation: a second press before React re-renders must step AGAIN,
// not restart from the value the first one saw.
panels = []; sets = [];
keyRef.current = { mdPreview: false, mdZoom: 100, appZoom: 1, switchOpen: false, settingsOpen: false };
const ev = { key: "+", metaKey: true, ctrlKey: false, altKey: false, preventDefault: () => {} };
onKey(ev); onKey(ev);
eq(sets, [{ app_zoom: 1.1 }, { app_zoom: 1.25 }], "a fast double-tap of ⌘+ walks two steps");

// Deliberately ungated: "make everything bigger" must work from inside a sheet.
eq(press("+", {}, { settingsOpen: true }).sets, [{ app_zoom: 1.1 }], "⌘+ works with Settings open");
eq(press("+", {}, { switchOpen: true }).sets, [{ app_zoom: 1.1 }], "⌘+ works with the ⌘K palette open");

// ⌥ switches to the markdown reader's own size — and only where one is showing.
const md = press("+", { alt: true }, { mdPreview: true });
eq(md.panels, [{ files_md_zoom: 110 }], "⌘⌥+ steps the markdown reader");
eq(md.sets, [], "⌘⌥+ leaves the app size alone");
eq(press("0", { alt: true }, { mdPreview: true, mdZoom: 150 }).panels, [{ files_md_zoom: 100 }], "⌘⌥0 resets the reader");
const noMd = press("+", { alt: true }, { mdPreview: false });
eq([noMd.panels, noMd.sets, noMd.prevented], [[], [], false], "⌘⌥+ with no rendered markdown does nothing at all");
eq(press("+", { alt: true }, { mdPreview: true, settingsOpen: true }).panels, [], "⌘⌥+ is still gated by Settings");

// macOS composes ⌥ with the layout: ⌥- is an en dash, ⌥= is "≠". The chord has
// to survive that, or ⌘⌥+/⌘⌥− are dead on every composing layout — which is
// every US Mac. Only `e.code` can see through it.
eq(press("–", { alt: true, code: "Minus" }, { mdPreview: true, mdZoom: 125 }).panels,
   [{ files_md_zoom: 110 }], "⌘⌥− arriving as an en dash still steps down");
eq(press("≠", { alt: true, code: "Equal" }, { mdPreview: true }).panels,
   [{ files_md_zoom: 110 }], "⌘⌥= arriving as ≠ still steps up");
eq(press("º", { alt: true, code: "Digit0" }, { mdPreview: true, mdZoom: 175 }).panels,
   [{ files_md_zoom: 100 }], "⌘⌥0 arriving composed still resets");

// The numeric keypad, whose `key` is unshifted but whose `code` is its own.
eq(press("+", { code: "NumpadAdd" }).sets, [{ app_zoom: 1.1 }], "keypad ⌘+ steps up");
eq(press("Clear", { code: "Numpad0" }, { appZoom: 2 }).sets, [{ app_zoom: 1 }], "keypad ⌘0 resets");

// Ctrl is not ⌘, and ⌘ plus a non-size key is somebody else's chord: both must
// FALL THROUGH to the meta-only section below, not be swallowed here.
const ctrl = press("+", { meta: false, ctrl: true, code: "Equal" });
eq([ctrl.sets, ctrl.prevented, ctrl.through], [[], false, true], "ctrl+ falls through, unhandled");
const other = press("k", { code: "KeyK" });
eq([other.sets, other.prevented, other.through], [[], false, true], "⌘K falls through to its own handler");

// ── the wiring the chord hangs off ─────────────────────────────────────────
// Everything above can pass with the feature entirely disconnected: delete the
// effect that pushes the factor to the webview, or drop `set_zoom` from
// `generate_handler!`, and tsc, cargo and every assertion here stay green —
// `applyZoom`'s `.catch()` swallows the rejected invoke, so the app is silent
// too. Three presence checks, because presence is all a static pass can know.
if (!/useEffect\(\(\) => \{ applyZoom\(settings\.app_zoom\); \}, \[settings\.app_zoom\]\);/.test(fs.readFileSync(APP, "utf8")))
  fail("App.tsx: the effect pushing app_zoom to the webview is gone — the chord would update a setting nothing applies");
if (!/\n\s+set_zoom,\n/.test(rs))
  fail("lib.rs: `set_zoom` is not in generate_handler! — every invoke rejects, and applyZoom swallows it");
const mock = read("../src/mock/install.ts");
if (!/case "set_zoom":/.test(mock))
  fail("mock/install.ts: no `set_zoom` case — the harness logs a rejected invoke on every ⌘+ (CLAUDE.md: the mock must track every command in lib.rs)");

if (bad) { console.error(`\nzoom-check: ${bad} failure(s)`); process.exit(1); }
console.log(`zoom-check: chord ok — ${BLOCK.split("\n").length}L of App.tsx driven, wiring present`);
