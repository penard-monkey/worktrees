// Pins the chat view's per-place choice, `PlacePanels.agent_view`: ABSENT
// MEANS TERMINAL, and nothing ever seeds it.
//
//   node app/scripts/placeview-check.mjs        # exits non-zero on failure
//
// Every other key in `place_panels` has a global twin that doubles as the
// seed for a place with no record (`panelsFor` returns `{...s, ...p}`). This
// one must not: a place you never switched opens on its Terminal, never on
// whatever the last place used. Four ways to break that, each of which
// type-checks and draws a perfectly good screen in the place you are looking
// at:
//
//  1. **A global appears.** `updatePanels` spreads the record it builds over
//     the globals, so carrying `agent_view` in THAT object makes a flat
//     `agent_view`, and the next reader that falls back to it (or the next
//     `panelsFor` spread) hands one place's choice to every other.
//  2. **It is dropped.** `updatePanels` rebuilds the record field by field, so
//     a field it does not name vanishes on the next dock toggle: Chat, then ⌘J,
//     and the place is back on its Terminal.
//  3. **It is filled in.** Writing `"terminal"` into every record a dock act
//     touches looks harmless and is the seed trap's first half: absent stops
//     meaning "never chose".
//  4. **The switch freezes the dock.** A place's first record, made by the
//     switch alone, must not copy the global `dock_tab`/`dock_width` in, or
//     that place stops inheriting them (AGENTS.md, "A `place_panels` field
//     whose global twin is a SEED must be optional").
//
// Like `dockrail-check.mjs` this loads the REAL `settings.ts`, and like
// `race-check.mjs` it runs the REAL `updatePanels`, sliced out of App.tsx and
// evaluated with stubs. A source that no longer matches its marker is a
// FAILURE, not a skipped assertion.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
const ok = (name, cond, detail = "") => {
  if (cond) return;
  failed++;
  console.log(`not ok — ${name}${detail ? `\n    ${detail}` : ""}`);
};

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const settingsSrc = read("../src/settings.ts");
const app = read("../src/App.tsx");

// ── the real settings.ts ────────────────────────────────────────────────────
// Inlined exactly as dockrail-check.mjs does it (a data: URL has no base for
// `./x`); the Tauri bridge answers `get_settings` with the blob under test and
// records what `set_settings` was handed.
const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
const stripImport = (src, mod) => {
  const re = new RegExp(`^import \\{[^}]*\\} from "\\./${mod}";$`, "m");
  ok(`settings.ts imports from ./${mod} as expected (the inlining is not stale)`, re.test(src));
  return src.replace(re, "");
};
const sSrc =
  inline("../src/afterglow.ts") + inline("../src/harness.ts") +
  stripImport(stripImport(settingsSrc, "afterglow"), "harness")
    .replace(/^import \{ invoke \}.*$/m,
      "const invoke = (cmd, args) => { if (cmd === \"set_settings\") globalThis.__saved = args.settings; " +
      "return Promise.resolve(cmd === \"get_settings\" ? structuredClone(globalThis.__blob) : undefined); };");
const S = await import("data:text/javascript;base64," + Buffer.from(
  (await transformWithEsbuild(sSrc, "settings.ts", { loader: "ts", format: "esm" })).code).toString("base64"));
for (const f of ["loadSettings", "saveSettings", "panelsFor", "placeKey", "agentViewFor", "withAgentView"])
  ok(`settings.ts exports ${f}`, typeof S[f] === "function");

// ── the real updatePanels ───────────────────────────────────────────────────
const lines = app.split("\n");
const from = lines.findIndex((l) => l.includes("const updatePanels = useCallback("));
ok("App.tsx still declares `const updatePanels = useCallback(`", from >= 0);
const to = lines.findIndex((l, i) => i > from && /^\s*\}, \[\]\);\s*$/.test(l));
ok("updatePanels still ends with its `}, []);`", to > from);
const UPDATE = lines.slice(from, to + 1).join("\n");
const uSrc = `
type DockPatch = any; type PlacePanels = any; type Settings = any;
export function build(env: any) {
  const { useCallback, setSettings, selRef, hydrated, preHydration, panelsFor, placeKey, applySettings, saveSettings } = env;
${UPDATE}
  return updatePanels;
}`;
const { build } = await import("data:text/javascript;base64," + Buffer.from(
  (await transformWithEsbuild(uSrc, "updatePanels.ts", { loader: "ts", format: "esm" })).code).toString("base64"));
/** A one-place App: `state` is `settings`, `sel` is the selected place. */
function world(state, slug, isHydrated = true) {
  const w = {
    state, saved: null,
    useCallback: (f) => f,
    setSettings: (fn) => { w.state = fn(w.state); },
    selRef: { current: { repo: "/r", slug } },
    hydrated: { current: isHydrated },
    preHydration: { current: {} },
    panelsFor: S.panelsFor, placeKey: S.placeKey,
    applySettings: () => {},
    saveSettings: (s) => { w.saved = s; },
  };
  w.update = build(w);
  return w;
}

const A = "/r|a", B = "/r|b", C = "/r|c", BAD = "/r|bad", NUM = "/r|num", BARE = "/r|bare";
const flat = (s) => { const { place_panels: _p, ...rest } = s; return rest; };

// ── 1. load: the two words survive, anything else is dropped, absent stays ──
globalThis.__blob = {
  settings_rev: 999, // current: `migrate` has nothing to do and saves nothing
  chat_view: true,
  dock_tab: "plan",
  dock_width: 520,
  a_newer_builds_key: "kept",
  place_panels: {
    [A]: { dock_open: true, dock_tab: "files", dock_width: 400, agent_view: "chat", a_newer_builds_field: 7 },
    [B]: { dock_open: false, dock_tab: "files", dock_width: 400, agent_view: "terminal" },
    [BAD]: { dock_open: false, dock_tab: "files", dock_width: 400, agent_view: "Chat" },
    [NUM]: { dock_open: false, agent_view: 1 },
    [BARE]: { dock_open: true, dock_tab: "docs", dock_width: 300 },
  },
};
const s = await S.loadSettings();
eq("a place left on Chat loads on Chat", s.place_panels[A].agent_view, "chat");
eq("a place left on Terminal loads on Terminal", s.place_panels[B].agent_view, "terminal");
eq("a misspelt view is dropped, not kept", "agent_view" in s.place_panels[BAD], false);
eq("a non-string view is dropped", "agent_view" in s.place_panels[NUM], false);
eq("a record with no view is not given one", "agent_view" in s.place_panels[BARE], false);
eq("…and its other fields are untouched", s.place_panels[BAD].dock_tab, "files");
eq("no global agent_view comes out of a load", "agent_view" in s, false);
// A save must not erase what it does not understand (AGENTS.md, "Retiring a
// key from ui-state.json breaks a DOWNGRADE"), nor the view itself.
S.saveSettings(s);
await new Promise((r) => setTimeout(r, 400)); // saveSettings debounces 250 ms
ok("saveSettings reached set_settings", !!globalThis.__saved);
globalThis.__blob = globalThis.__saved ?? {};
const s2 = await S.loadSettings();
eq("Chat survives a save and a reload", s2.place_panels?.[A]?.agent_view, "chat");
eq("Terminal survives a save and a reload", s2.place_panels?.[B]?.agent_view, "terminal");
eq("an unknown per-place field survives a save", s2.place_panels?.[A]?.a_newer_builds_field, 7);
eq("an unknown global survives a save", s2.a_newer_builds_key, "kept");

// ── 2. read: the record alone, Terminal when absent ─────────────────────────
eq("agentViewFor: Chat", S.agentViewFor(s, A), "chat");
eq("agentViewFor: Terminal", S.agentViewFor(s, B), "terminal");
eq("agentViewFor: no record is Terminal", S.agentViewFor(s, C), "terminal");
eq("agentViewFor: a record with no view is Terminal", S.agentViewFor(s, BARE), "terminal");
eq("agentViewFor: no place is Terminal", S.agentViewFor(s, null), "terminal");
eq("agentViewFor: a global cannot seed a place that never chose",
  S.agentViewFor({ ...s, agent_view: "chat" }, C), "terminal");
eq("agentViewFor does not depend on chat_view (the choice is KEPT while it is off)",
  S.agentViewFor({ ...s, chat_view: false }, A), "chat");
eq("panelsFor leaves agent_view behind (a record that has one)", "agent_view" in S.panelsFor(s, A), false);
eq("panelsFor leaves agent_view behind (no record)", "agent_view" in S.panelsFor(s, C), false);

// ── 3. write: withAgentView touches one place's view and nothing else ──────
const n1 = S.withAgentView(s, C, "chat");
eq("a first switch makes a record with a closed dock and the view, nothing else",
  n1.place_panels[C], { dock_open: false, agent_view: "chat" });
eq("…so that place still inherits the dock tab", S.panelsFor(n1, C).dock_tab, s.dock_tab);
eq("…and the dock width", S.panelsFor(n1, C).dock_width, s.dock_width);
eq("…and its dock is still closed", S.panelsFor(n1, C).dock_open, false);
ok("every other place's record is the same object",
  Object.keys(s.place_panels).every((k) => n1.place_panels[k] === s.place_panels[k]));
eq("no global changes", flat(n1), flat(s));
eq("a switch in one place leaves a never-touched place on Terminal", S.agentViewFor(n1, "/r|d"), "terminal");
ok("Terminal in a place that never chose writes nothing", S.withAgentView(s, C, "terminal") === s);
ok("Chat in a place already on Chat writes nothing", S.withAgentView(s, A, "chat") === s);
const n2 = S.withAgentView(s, A, "terminal");
eq("back to Terminal keeps the rest of the record", n2.place_panels[A], { ...s.place_panels[A], agent_view: "terminal" });

// ── 4. updatePanels carries it, never invents it, never globalises it ──────
const wa = world(s, "a");
wa.update({ dock_open: false });
eq("a dock act keeps the place's Chat", wa.state.place_panels[A].agent_view, "chat");
wa.update((cur) => ({ dock_open: !cur.dock_open }));
eq("…through the function form too (⌘J)", wa.state.place_panels[A].agent_view, "chat");
eq("…and the save carries it", wa.saved?.place_panels?.[A]?.agent_view, "chat");
eq("a dock act makes no global agent_view", "agent_view" in wa.state, false);
const wb = world(s, "b");
wb.update({ dock_tab: "plan" });
eq("a place on Terminal stays on Terminal", wb.state.place_panels[B].agent_view, "terminal");
const wc = world(wa.state, "c");
wc.update({ dock_open: true, dock_tab: "docs" });
eq("a dock act in a place that never chose writes no view", "agent_view" in wc.state.place_panels[C], false);
eq("…so it reads Terminal after a place on Chat was written", S.agentViewFor(wc.state, C), "terminal");
eq("…and still no global agent_view", "agent_view" in wc.state, false);
const wn = world(n1, "c");
wn.update({ dock_open: true });
eq("a dock act after a first switch keeps that view", wn.state.place_panels[C].agent_view, "chat");
eq("…and now the record holds the full dock", wn.state.place_panels[C].dock_tab, s.dock_tab);
const wp = world(s, "a", false);
wp.update({ dock_open: true });
eq("pre-hydration, the stash never holds a view", "agent_view" in wp.preHydration.current, false);

// ── 5. App.tsx reads it in one place and writes it in one place ────────────
const chatOn = [...app.matchAll(/const chatOn = ([^;\n]+);/g)];
eq("one `const chatOn =` in App.tsx", chatOn.length, 1);
ok("chatOn is chat_view AND the place's own view",
  chatOn.length === 1 && /^settings\.chat_view && agentViewFor\(settings, viewKey\) === "chat"$/.test(chatOn[0][1]),
  `found: ${chatOn[0]?.[1]}`);
const sv = lines.findIndex((l) => l.includes("const setChatView = useCallback("));
const svEnd = lines.findIndex((l, i) => i > sv && /^\s*\}, \[\]\);\s*$/.test(l));
ok("App.tsx still declares `const setChatView = useCallback(`", sv >= 0 && svEnd > sv);
ok("setChatView writes through withAgentView", lines.slice(sv, svEnd).join("\n").includes("withAgentView(prev, key,"));
// Any other CODE line naming the field is a second reader or writer, which is
// where a fallback to a global would come back in.
const isComment = (l) => /^\s*(\/\/|\*|\/\*)/.test(l);
const stray = lines
  .map((l, i) => ({ l, i }))
  .filter(({ l, i }) => l.includes("agent_view") && !isComment(l) && !(i >= from && i <= to)
    && !l.includes('Omit<PlacePanels, "agent_view">'));
eq("App.tsx names agent_view in code only inside updatePanels", stray.map(({ l, i }) => `${i + 1}: ${l.trim()}`), []);

if (failed) {
  console.log(`\n${failed} failure(s)`);
  process.exit(1);
}
console.log("ok — agent_view: absent is Terminal, never seeded, carried by updatePanels, survives load/save");
