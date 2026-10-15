// Guards Settings → Layout: the two sides (`places_side`, `dock_side`), the
// rev-4 migration that split them, and the merged rail they imply.
//
//   node app/scripts/layout-check.mjs        # exits non-zero on failure
//
// Four things here can be wrong while every other suite passes:
//
//  1. **The migration asks the wrong question.** `loadSettings` spreads the
//     stored blob over DEFAULTS BEFORE `migrate` runs, so `dock_side` is always
//     present by then. A step written as "if the key is missing" never fires,
//     and every mirrored install upgrades to `dock_side: "right"` — its dock
//     jumps beside its nav and the rails merge. Run against the REAL
//     `loadSettings` with stored blobs, not read as text, so any phrasing of
//     that mistake fails.
//  2. **The grid and its children disagree.** `shell` (App.tsx) is the column
//     list AND each child's `order`. The merged shapes have three tracks and no
//     `railRight`, so a child reading `ord.railRight` outside `!merged` would get
//     `undefined` — no error, just a rail auto-placed into the wrong column.
//  3. **The dock rail renders twice, or not at all.** `DockRailGroup` and
//     `OffersRailButton` each have two hosts; they must be under complementary
//     `merged` conditions, or the buttons appear in both rails (or neither).
//  4. **The tiles drift from the sides.** The four tiles must be the 2×2 exactly
//     once, and what each tile says about the window must be what App builds.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

const ts = read("../src/settings.ts");
const app = read("../src/App.tsx");
const css = read("../src/App.css");

// ── load the real settings.ts (same inlining as zoom-check.mjs) ──────────────
// A data: URL has no base for `./afterglow`, so those modules' REAL source is
// inlined ahead of it; `invoke` is the only stub, and it is the knob: it hands
// `loadSettings` whatever blob a case says is on disk.
let stored = null;
const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
const stripImport = (src, mod) => {
  const re = new RegExp(`^import \\{[^}]*\\} from "\\./${mod}";$`, "m");
  if (!re.test(src)) fail(`settings.ts: no \`import { … } from "./${mod}"\` line — the inlining below is stale`);
  return src.replace(re, "");
};
globalThis.__layoutInvoke = (cmd) => Promise.resolve(cmd === "get_settings" ? structuredClone(stored) : null);
const src =
  inline("../src/afterglow.ts") + inline("../src/harness.ts") +
  stripImport(stripImport(ts, "afterglow"), "harness")
    .replace(/^import \{ invoke \}.*$/m, "const invoke = (cmd, a) => globalThis.__layoutInvoke(cmd, a);");
const js = (await transformWithEsbuild(src, "settings.ts", { loader: "ts", format: "esm" })).code;
const S = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── 1. the migration, through the real loadSettings ─────────────────────────
const cases = [
  // [what is on disk, expected places_side, expected dock_side, why]
  [null, "left", "right", "a fresh install is Classic"],
  [{ places_side: "left", settings_rev: 3 }, "left", "right", "a v0.41 default install stays Classic"],
  [{ places_side: "right", settings_rev: 3 }, "right", "left", "a v0.41 MIRRORED install stays Mirrored (dock opposite its nav, rails apart)"],
  [{ places_side: "right" }, "right", "left", "a pre-rev install that mirrored stays Mirrored"],
  [{ places_side: "up", settings_rev: 3 }, "left", "right", "a junk places_side normalises before the dock's side is derived from it"],
  [{ places_side: "left", dock_side: "left", settings_rev: 4 }, "left", "left", "a rev-4 choice of Agent right is kept, not re-derived"],
  [{ places_side: "right", dock_side: "right", settings_rev: 4 }, "right", "right", "a rev-4 choice of Agent left is kept"],
  [{ places_side: "left", dock_side: "middle", settings_rev: 4 }, "left", "right", "a junk dock_side normalises to the default edge"],
];
for (const [blob, p, d, why] of cases) {
  stored = blob;
  const s = await S.loadSettings();
  if (s.places_side !== p || s.dock_side !== d) {
    fail(`${why}: stored ${JSON.stringify(blob)} loaded as places=${s.places_side} dock=${s.dock_side}, expected ${p}/${d}`);
  } else ok(why);
}
if (S.SETTINGS_REV < 4) fail(`SETTINGS_REV is ${S.SETTINGS_REV} — the dock_side step needs 4`);

// ── geometry: merged frees one rail, and only the merged shapes do ──────────
{
  const base = { ...S.DEFAULTS, nav_pinned: true, dock_open: true, nav_width: 300, dock_width: 360 };
  const w = 1440;
  const mains = Object.fromEntries(S.LAYOUT_TILES.map((t) =>
    [t.name, S.fitLayout({ ...base, places_side: t.places, dock_side: t.dock }, true, w).mainW]));
  const want = w - 300 - 360;
  const off = S.LAYOUT_TILES.filter((t) => mains[t.name] !== want - (S.railsMerged({ places_side: t.places, dock_side: t.dock }) ? S.RAIL_W : 2 * S.RAIL_W));
  if (off.length) fail(`fitLayout's main width is off for ${off.map((t) => `${t.name}=${mains[t.name]}`).join(", ")} (want ${want} minus one rail merged, two apart)`);
  else ok("fitLayout reserves one rail for the merged shapes and two otherwise");
}

// ── 4. the tiles are the 2×2, once each ─────────────────────────────────────
{
  const T = S.LAYOUT_TILES;
  const pairs = T.map((t) => `${t.places}/${t.dock}`);
  const all = ["left/right", "left/left", "right/left", "right/right"];
  if (T.length !== 4 || new Set(pairs).size !== 4 || !all.every((p) => pairs.includes(p))) {
    fail(`LAYOUT_TILES is ${JSON.stringify(pairs)} — want each of ${all.join(", ")} exactly once`);
  } else ok("the four tiles are the 2×2 of the two sides, once each");
  const names = Object.fromEntries(T.map((t) => [`${t.places}/${t.dock}`, t.name]));
  const wantNames = { "left/right": "Classic", "left/left": "Agent right", "right/left": "Mirrored", "right/right": "Agent left" };
  for (const [k, n] of Object.entries(wantNames)) if (names[k] !== n) fail(`the ${k} tile is named "${names[k]}", want "${n}"`);
  // What a tile CLAIMS about the window must be what App builds: nav at the
  // Places edge, reference on the dock's side of the agent.
  for (const t of T) {
    const o = t.order;
    const navOk = t.places === "left" ? o[0] === "nav" : o[o.length - 1] === "nav";
    const refOk = (o.indexOf("reference") < o.indexOf("agent")) === (t.dock === "left");
    if (!navOk || !refOk) fail(`the ${t.name} tile says ${o.join(" · ")} for places=${t.places} dock=${t.dock}`);
  }
  if (!bad) ok("each tile's caption matches the sides it writes");
}

// ── 2. the shell builder, evaluated for each tile ───────────────────────────
{
  const from = app.indexOf("  const navTrack: [string, string | null]");
  const to = from < 0 ? -1 : app.indexOf("  const ord = Object.fromEntries(shell", from);
  if (from < 0 || to < 0) fail("App.tsx: the `navTrack … const ord` block is gone — this check evaluates the shell builder from it");
  else {
    const block = app.slice(from, to).replace(/: \[string, string \| null\]/g, "").replace(/: \[key: string, track: string \| null\]\[\]/, "");
    // eslint-disable-next-line no-new-func
    const build = new Function("merged", "mirrored", "fit", `${block}; return shell.map(([k]) => k);`);
    for (const t of S.LAYOUT_TILES) {
      const merged = S.railsMerged({ places_side: t.places, dock_side: t.dock });
      const got = build(merged, t.places === "right", { navShown: true, navW: 300 });
      // The tile's regions, with the reference folded into `space` (the dock
      // is inside it) and the rails added at the edges App owns them.
      const regions = t.order.map((r) => (r === "nav" ? "nav" : "space")).filter((r, i, a) => r !== a[i - 1]);
      const want = t.places === "left"
        ? ["rail", ...regions, ...(merged ? [] : ["railRight"])]
        : [...(merged ? [] : ["railRight"]), ...regions, "rail"];
      if (JSON.stringify(got) !== JSON.stringify(want)) fail(`${t.name}: the grid is ${got.join(" | ")}, want ${want.join(" | ")}`);
    }
    if (!bad) ok("the grid's columns match every tile (three tracks merged, four apart)");
  }
  // Every `ord.X` a child reads must exist in every shape where that child
  // renders. `railRight` is absent from the merged shapes, so its one reader
  // must sit inside `!merged`.
  const reads = [...app.matchAll(/order: ord\.(\w+)/g)].map((m) => [m[1], m.index]);
  for (const [k, at] of reads) {
    if (!["rail", "nav", "space", "railRight"].includes(k)) fail(`App.tsx reads ord.${k}, which no shape defines`);
    if (k === "railRight" && !/\{!merged && \(\s*$/.test(app.slice(Math.max(0, at - 200), at).split("<nav")[0])) {
      fail("App.tsx: the dock rail reads `ord.railRight` outside a `{!merged && (` — the merged shapes have no such track");
    }
  }
  if (!reads.some(([k]) => k === "railRight")) fail("App.tsx no longer orders the dock rail with ord.railRight");
  // The dock's own side inside `.space`: a flex `order`, keyed on dock_side.
  if (!/<aside [^>]*className=\{?"dock"[^>]*style=\{\{ order: dockLeft \? -1 : 0/.test(app)) {
    fail("App.tsx: `.dock`'s order is not `dockLeft ? -1 : 0` — the dock no longer follows dock_side");
  } else ok("the dock's position inside .space follows dock_side");
  if (!/const dockLeft = settings\.dock_side === "left";/.test(app) || !/const merged = railsMerged\(settings\);/.test(app)) {
    fail("App.tsx: `dockLeft` / `merged` are not derived from settings.dock_side / railsMerged(settings)");
  }
  if (!/dockLeft \? ev\.clientX - startX : startX - ev\.clientX/.test(app)) {
    fail("App.tsx: the dock resizer's sign is not keyed on dockLeft — pulling it open would shrink it on one side");
  } else ok("the dock resizer's sign follows dock_side");
}

// ── the notice stack sits over the AGENT in every layout ─────────────────────
// `leftOfMain` is evaluated against the grid this file just checked: the
// tracks before `space`, plus the dock when it is on the left and open.
{
  const m = app.match(/const leftOfMain = ([\s\S]*?);\n/);
  const from = app.indexOf("  const navTrack: [string, string | null]");
  const to = app.indexOf("  const ord = Object.fromEntries(shell", from);
  if (!m || from < 0) fail("App.tsx: `const leftOfMain = …;` is gone — where does the notice stack anchor?");
  else if (!/left: `calc\(\$\{leftOfMain\}px \+ var\(--s3\)\)`/.test(app)) fail("`.float-stack` does not read leftOfMain");
  else {
    const block = app.slice(from, to).replace(/: \[string, string \| null\]/g, "").replace(/: \[key: string, track: string \| null\]\[\]/, "");
    // eslint-disable-next-line no-new-func
    const build = new Function("merged", "mirrored", "fit", `${block}; return shell;`);
    // eslint-disable-next-line no-new-func
    const left = new Function("mirrored", "merged", "fit", "dockLeft", "dockShown", "RAIL_W", `return ${m[1]};`);
    const wrong = [];
    for (const t of S.LAYOUT_TILES) for (const navShown of [false, true]) for (const dockShown of [false, true]) {
      const merged = t.places === t.dock, mirrored = t.places === "right", dockLeft = t.dock === "left";
      const fit = { navShown, navW: 300, dockW: 360 };
      const shell = build(merged, mirrored, fit);
      const tracks = shell.slice(0, shell.findIndex(([k]) => k === "space")).reduce((a, [, tr]) => a + (tr === "var(--rail-w)" ? S.RAIL_W : tr ? parseInt(tr) : 0), 0);
      const want = tracks + (dockLeft && dockShown ? 360 : 0);
      const got = left(mirrored, merged, fit, dockLeft, dockShown, S.RAIL_W);
      if (got !== want) wrong.push(`${t.name} nav ${navShown ? "pinned" : "hidden"} dock ${dockShown ? "open" : "shut"}: ${got} vs ${want}`);
    }
    if (wrong.length) fail(`the notice stack is not at the agent's left edge: ${wrong.join("; ")}`);
    else ok("the notice stack sits at the agent's bottom-left in all 16 shapes");
  }
}

// ── 3. one dock group, two hosts, complementary conditions ──────────────────
// Sliced by HOST rather than by a window of characters before each call: the
// Places rail (`<nav className="rail"` … its `</nav>`) and the dock's own rail,
// which must sit inside a `{!merged && (`.
{
  const span = (open) => {
    const a = app.indexOf(open);
    return a < 0 ? null : [a, app.indexOf("</nav>", a)];
  };
  const places = span('<nav\n        className="rail"\n');
  const dock = span('<nav className="rail rail-right"');
  if (!places) fail("App.tsx: the Places rail (`<nav className=\"rail\"` on its own line) is gone — renamed?");
  if (!dock) fail("App.tsx: the dock rail (`<nav className=\"rail rail-right\"`) is gone — renamed?");
  if (places && dock) {
    if (!/\{!merged && \(\s*$/.test(app.slice(dock[0] - 40, dock[0]))) fail("App.tsx: the dock rail is not wrapped in `{!merged && (` — merged, its buttons would show twice");
    const within = ([a, b], i) => i > a && i < b;
    for (const name of ["DockRailGroup", "OffersRailButton"]) {
      const at = [...app.matchAll(new RegExp(`<${name} `, "g"))].map((m) => m.index);
      const inPlaces = at.filter((i) => within(places, i));
      const inDock = at.filter((i) => within(dock, i));
      if (at.length !== 2 || inPlaces.length !== 1 || inDock.length !== 1) {
        fail(`${name}: ${at.length} renders, ${inPlaces.length} in the Places rail, ${inDock.length} in the dock rail — want 2, 1, 1`);
        continue;
      }
      // In the Places rail it must be gated on `merged` (no `!`): the nearest
      // opening brace's condition, read back from the call.
      // JSX comments are braces too; they gate nothing.
      const before = app.slice(places[0], inPlaces[0]).replace(/\{\/\*[\s\S]*?\*\/\}/g, "");
      const gate = before.slice(before.lastIndexOf("{") + 1);
      if (!/^merged &&/.test(gate.trim())) fail(`${name} in the Places rail is gated on \`${gate.trim().slice(0, 60)}\`, not \`merged && …\``);
    }
    // Merged, the dock's buttons sit in the auto-hide nav's hover edge. They
    // must be wrapped in `.rail-group` (hairline included) and the rail's
    // handler must skip that band — or reaching for Terminal slides Places
    // over the reference panel. The handler is EVALUATED, not read: a version
    // that never calls `navPointer` (hover-reveal dead for the whole rail)
    // reads exactly like a correct one.
    const placesRail = app.slice(places[0], places[1]);
    if (!/<div className="rail-group">[\s\S]*?<div className="rail-sep" \/>\s*<DockRailGroup /.test(placesRail)) {
      fail("the merged DockRailGroup (and its hairline) are not inside `.rail-group` — the hover edge cannot tell them apart");
    }
    const hAt = placesRail.indexOf("onPointerOver={(e) => {");
    const hEnd = hAt < 0 ? -1 : placesRail.indexOf("\n        }}", hAt);
    if (hAt < 0 || hEnd < 0) fail("the Places rail has no `onPointerOver={(e) => { … }}` — what decides the hover edge now?");
    else {
      const body = placesRail.slice(hAt + "onPointerOver={(e) => {".length, hEnd);
      const ElementStub = class {};
      const el = (inGroup) => Object.assign(new ElementStub(), { closest: (q) => (q === ".rail-group" && inGroup ? {} : null) });
      // eslint-disable-next-line no-new-func
      const handler = new Function("Element", "navOver", "navPointer", "e", body);
      const run = (inGroup, railBefore) => {
        const calls = [];
        const navOver = { current: { rail: railBefore, nav: false } };
        // The real navPointer records the flag first (App.tsx); mirror that.
        const navPointer = (which, inside) => { calls.push(`${which}:${inside}`); navOver.current[which] = inside; };
        handler(ElementStub, navOver, navPointer, { target: el(inGroup) });
        return calls.join(",");
      };
      const cases = [
        [false, false, "rail:true", "entering the rail (outside the group) reveals"],
        [true, true, "rail:false", "moving into the dock group leaves the hover edge"],
        [true, false, "", "inside the group, nothing re-arms"],
        [false, true, "", "moving within the edge does not re-reveal per element"],
      ];
      const wrong = cases.filter(([g, r, want]) => run(g, r) !== want);
      if (wrong.length) fail(`the rail's hover handler: ${wrong.map(([g, r, want, why]) => `${why} — got "${run(g, r)}", want "${want}"`).join("; ")}`);
      else ok("the rail's hover handler reveals outside the dock group and only there (evaluated)");
    }
    if (!bad) ok("DockRailGroup and the offers button render in exactly one rail for any layout");
  }
}

// ── CSS: the dock half keys on its own side ──────────────────────────────────
{
  if (!/root\.dataset\.dockSide = /.test(ts)) fail("applySettings no longer writes data-dock-side");
  for (const sel of [".dock {", ".dock-resizer {", ".rail-right {", ".rail-right .rail-icon.active::before {"]) {
    const placesKeyed = css.includes(`html[data-places-side="right"] ${sel}`);
    const dockKeyed = css.includes(`html[data-dock-side="left"] ${sel}`);
    if (placesKeyed || !dockKeyed) fail(`App.css: \`${sel.replace(" {", "")}\` is keyed on ${placesKeyed ? "data-places-side" : "nothing"} — the dock half follows data-dock-side`);
  }
  if (!bad) ok("the dock's border, resizer and rail rules key on data-dock-side");
}

// ── 5. the lower slot (phase 2) ──────────────────────────────────────────────
// The Terminal can sit under the reference panel instead of being a tab. The
// rule that matters: the same shells are NEVER mounted twice (a pty has one
// size; two xterms on it SIGWINCH each other), so the upper half must not be
// able to show the section the lower slot holds.
{
  for (const [blob, want, why] of [
    [{ dock_lower: "terminal", settings_rev: 4 }, "terminal", "the Terminal in the lower slot loads"],
    [{ dock_lower: "docs", settings_rev: 4 }, null, "a section the slot does not take yet loads as no slot"],
    [{ settings_rev: 3 }, null, "an older install has no lower slot"],
  ]) {
    stored = blob;
    const s = await S.loadSettings();
    if (s.dock_lower !== want) fail(`${why}: ${JSON.stringify(blob)} loaded dock_lower=${JSON.stringify(s.dock_lower)}`);
    else ok(why);
  }
  for (const [frac, want] of [[2, S.DOCK_LOWER_FRAC_MAX], [0, S.DOCK_LOWER_FRAC_MIN], ["x", S.DEFAULTS.dock_lower_frac], [0.5, 0.5]]) {
    stored = { dock_lower_frac: frac, settings_rev: 4 };
    const s = await S.loadSettings();
    if (s.dock_lower_frac !== want) fail(`dock_lower_frac ${JSON.stringify(frac)} loaded as ${s.dock_lower_frac}, want ${want}`);
  }
  if (S.upperTab("terminal", "terminal") !== "files" || S.upperTab("docs", "terminal") !== "docs" || S.upperTab("terminal", null) !== "terminal") {
    fail("upperTab lets the upper half show the section in the lower slot");
  } else ok("the upper half never shows the section in the lower slot");
  // The 560px window minimum: header + strip leave a ~490px column.
  if (!S.lowerFits(490)) fail("the slot does not fit the ~490px column at the 560px window minimum");
  if (S.lowerFits(250)) fail("lowerFits accepts a 250px column — the Terminal must fall back to a tab there");
  // The splitter's range is the COLUMN's, not the global 0.25..0.6: every
  // split it can release onto must hold both floors, so a drag can never be
  // what turns the slot into a tab (there is no control to bring it back).
  for (const h of [300, 490, 560, 800, 1400]) {
    const r = S.lowerRange(h);
    if (!r) { fail(`lowerRange(${h}) is empty, but lowerFits(${h}) is ${S.lowerFits(h)}`); continue; }
    for (const f of [r[0], r[1], (r[0] + r[1]) / 2]) {
      if (h * f < S.DOCK_LOWER_MIN_H - 1e-9 || h * (1 - f) < S.DOCK_UPPER_MIN_H - 1e-9) fail(`lowerRange(${h}) allows ${f.toFixed(3)}, which breaks a floor`);
    }
    for (const stored of [0, 0.25, 0.4, 0.6, 1]) {
      const d = S.lowerFracFor(h, stored);
      if (d < r[0] - 1e-9 || d > r[1] + 1e-9) fail(`a stored ${stored} is drawn at ${d.toFixed(3)} in a ${h}px column, outside ${r.map((x) => x.toFixed(3)).join("..")}`);
    }
  }
  const split = app.slice(app.indexOf("const onLowerSplit"), app.indexOf("// ── row ──"));
  if (!/const range = lowerRange\(r\.height\);/.test(split) || /clampLowerFrac\(/.test(split)) fail("the splitter is clamped to the global range, not the column's (`lowerRange`) — a release at the window minimum can hide the slot");
  else if (!bad) ok("the splitter can only release onto splits the column holds; a stored split is drawn in range");

  // The derivation, EVALUATED (navwire-check's shape). Over every
  // lowerSet × dockShown × fits × session state with a stored "terminal" tab:
  // the shells are never in both hosts, the slot is on screen only where it
  // fits and has shells, and wherever it is not, the Terminal is a TAB again.
  {
    const a = app.indexOf("  const lowerSet = settings.dock_lower");
    const b = app.indexOf("\n", app.indexOf("  const shownTab = upperTab(", a));
    if (a < 0 || b < 0) fail("App.tsx: `const lowerSet = …` … `const shownTab = …` is gone — this check evaluates it");
    else {
      const block = app.slice(a, b).replace(/<[A-Za-z|"\[\] ]+>/g, "").replace(/: Settings\["dock_lower"\]/g, "").replace(/\(el: HTMLElement \| null\)/g, "(el)");
      const js = (await transformWithEsbuild(`export function build(env) { const { settings, dockShown, selected, eff, lowerFits, upperTab, useState, useRef, useCallback, ResizeObserver } = env;\n${block}\n return { lowerOn, shownTab, dockLower }; }`, "derive.ts", { loader: "ts", format: "esm" })).code;
      const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
      const wrong = [];
      for (const lowerSet of [false, true]) for (const dockShown of [false, true]) for (const fits of [false, true]) for (const up of [false, true]) {
        const h = fits ? 490 : 250;
        const r = build({
          settings: { dock_lower: lowerSet ? "terminal" : null }, dockShown, selected: { tmux_session: { up } }, eff: { dock_tab: "terminal" },
          lowerFits: S.lowerFits, upperTab: S.upperTab, useState: () => [h, () => {}], useRef: () => ({ current: null }), useCallback: (f) => f, ResizeObserver: class {},
        });
        const tag = `set=${lowerSet} shown=${dockShown} fits=${fits} up=${up}`;
        if (r.lowerOn && r.shownTab === "terminal") wrong.push(`${tag}: shells in BOTH hosts`);
        if (r.lowerOn !== (lowerSet && dockShown && fits && up)) wrong.push(`${tag}: lowerOn=${r.lowerOn}`);
        if (dockShown && !r.lowerOn && r.shownTab !== "terminal") wrong.push(`${tag}: the Terminal is in neither host (upper shows ${r.shownTab})`);
      }
      if (wrong.length) fail(`the lower-slot derivation: ${wrong.join("; ")}`);
      else ok("the lower-slot derivation (evaluated, 16 states): one host at a time, and the Terminal is never lost");
    }
  }
  // Only the stored-value bookkeeping may read `eff.dock_tab`; everything that
  // asks "what is on screen" reads `shownTab` (the reader's guard once read
  // the preference instead, and kept a reader up over the Terminal tab).
  const readers = app.split("\n").map((l, i) => [l, i]).filter(([l]) => /eff\.dock_tab/.test(l));
  const allowed = (l) => /const shownTab = upperTab\(eff\.dock_tab/.test(l) || /prsGone/.test(l) || /d\.tab !== eff\.dock_tab/.test(l);
  const stray = readers.filter(([l]) => !allowed(l));
  if (stray.length) fail(`App.tsx reads the STORED tab where it means the shown one: ${stray.map(([l, i]) => `${i + 1}: ${l.trim()}`).join(" | ")}`);
  else ok("only shownTab's definition and the stored-tab bookkeeping read eff.dock_tab");

  const body = (() => {
    const a = app.indexOf('<div className="dock-body">');
    const b = app.indexOf("{/* ── the lower slot", a);
    return a < 0 || b < 0 ? "" : app.slice(a, b);
  })();
  const lower = (() => {
    const a = app.indexOf('<div className="dock-lower"');
    return a < 0 ? "" : app.slice(a, app.indexOf("</div>", a));
  })();
  const terms = [...app.matchAll(/<TerminalTabs /g)].length;
  if (!body || !lower) fail("App.tsx: the dock body or the `.dock-lower` slot is gone — renamed?");
  else {
    if (terms !== 2 || !body.includes("<TerminalTabs ") || !lower.includes("<TerminalTabs ")) fail(`TerminalTabs renders ${terms} times — want one in the tab chain, one in the lower slot`);
    if (/eff\.dock_tab/.test(body)) fail("the dock body branches on `eff.dock_tab` — with the Terminal below, a stored \"terminal\" tab would mount the shells twice; branch on `shownTab`");
    if (!/const shownTab = upperTab\(eff\.dock_tab, dockLower\);/.test(app)) fail("`shownTab` is not `upperTab(eff.dock_tab, dockLower)`");
    if (!/\{lowerOn && \(/.test(app.slice(app.indexOf("{/* ── the lower slot"), app.indexOf('<div className="dock-lower"')))) fail("the lower slot is not gated on `lowerOn`");
    if (!/findOpen=\{findOn === "lower"\}/.test(lower)) fail("the lower shells answer the DOCK's find bar — ⌘F would open two bars");
    if (!/focusToken=\{termFocus \+ lowerFocus\}/.test(lower)) fail("the lower shells cannot be focused on their own (the rail icon's click)");
    if (!bad) ok("the lower slot hosts the one other TerminalTabs, on its own find surface and focus token");
  }
  // The reader covers the whole column, lower slot included: opening it must
  // close the lower shell's find bar too, or two bars are live at once.
  if (!/useEffect\(\(\) => \{ if \(reading\) setFindOn\(\(f\) => \(f === "dock" \|\| f === "lower" \? null : f\)\); \}, \[reading\]\);/.test(app)) {
    fail("opening the reader does not close the lower shell's find bar");
  }
  if (!/const stripUp = dockShown && \(shownTab === "terminal" \|\| lowerOn\);/.test(app)) fail("navHistory's `stripUp` ignores the lower strip — a location would record no front shell");
  // The notice stack's corner would be the lower shell's prompt with the
  // column on the left; `leftOfMain` (checked above, all 16 shapes) already
  // puts it past an open left-hand column, which is every case with a slot.
  const lowerCss = css.match(/\.dock-lower \{[^}]*\}/)?.[0] ?? "";
  if (!/min-height: 0/.test(lowerCss)) fail("App.css: `.dock-lower` has no `min-height: 0` — the shell's grid becomes a floor the slot never gives back");
  else ok("`.dock-lower` can shrink (min-height: 0)");
}

// ── 6. drag to arrange (phase 3) ────────────────────────────────────────────
// arrange.ts is pure (type-only imports), so it loads on its own. What can go
// wrong: a destination writing a field Undo/Reset do not know, two zones under
// one pointer, the agent's grip writing its OWN side backwards, and a drag that
// writes settings while it moves (a live reflow resizes the agent per frame).
{
  const asrc = read("../src/arrange.ts");
  const ajs = (await transformWithEsbuild(asrc, "arrange.ts", { loader: "ts", format: "esm" })).code;
  const A = await import("data:text/javascript;base64," + Buffer.from(ajs).toString("base64"));
  const grips = ["places", "reference", "agent", "terminal"];
  const geo = { app: { left: 0, top: 0, right: 1280, bottom: 800 }, space: { left: 344, top: 50, right: 1280, bottom: 800 }, dock: { left: 344, top: 50, right: 704, bottom: 800 } };
  for (const g of grips) {
    const ds = A.destinations(g);
    if (ds.length !== 2) fail(`${g}: ${ds.length} destinations, want 2`);
    for (const d of ds) {
      const extra = Object.keys(d.patch).filter((k) => !A.LAYOUT_KEYS.includes(k));
      if (extra.length) fail(`${g} → ${d.id} writes ${extra.join(", ")}, which Undo and Reset do not cover`);
      const z = A.zoneOf(d.id, geo);
      if (!z) { fail(`${g} → ${d.id} has no zone with the column open`); continue; }
      const cx = (z.left + z.right) / 2, cy = (z.top + z.bottom) / 2;
      if (A.destAt(g, geo, cx, cy)?.id !== d.id) fail(`${g}: the centre of ${d.id}'s zone resolves to ${A.destAt(g, geo, cx, cy)?.id}`);
    }
    const [a, b] = ds.map((d) => A.zoneOf(d.id, geo));
    if (a && b && a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom) fail(`${g}: its two zones overlap`);
  }
  if (A.zoneOf("term-below", { ...geo, dock: null }) !== null) fail("the Terminal's zones exist with the column closed");
  // The agent's grip writes the REFERENCE side, inverted.
  const ag = Object.fromEntries(A.destinations("agent").map((d) => [d.id, d.patch.dock_side]));
  if (ag["agent-left"] !== "right" || ag["agent-right"] !== "left") fail(`dragging the agent writes dock_side backwards (${JSON.stringify(ag)})`);
  // Where the agent zone sits must agree with what the patch produces: the
  // agent dropped in the LEFT half means the reference is on the right.
  const cur = { places_side: "left", dock_side: "left", dock_lower: null };
  const here = A.destinations("agent").filter((d) => A.isCurrent(d, cur)).map((d) => d.id);
  if (JSON.stringify(here) !== '["agent-right"]') fail(`with reference left, the agent's current zone reads as ${JSON.stringify(here)}`);
  const u = A.undoOf({ dock_side: "right", dock_lower: "terminal" }, cur);
  if (JSON.stringify(u) !== JSON.stringify({ dock_side: "left", dock_lower: null })) fail(`undoOf captured ${JSON.stringify(u)}`);
  const r = A.resetPatch(S.DEFAULTS);
  if (JSON.stringify(r) !== JSON.stringify({ places_side: "left", dock_side: "right", dock_lower: null })) fail(`resetPatch is ${JSON.stringify(r)}, not the defaults of the three layout keys`);
  if (!bad) ok("arrange.ts: two destinations per grip, disjoint zones, layout keys only, agent inverted, undo/reset exact");

  // App: four grips; the drag paints and never writes until the drop.
  const gripUses = [...app.matchAll(/\{\.\.\.gripProps\("(\w+)"\)\}/g)].map((m) => m[1]).sort();
  if (JSON.stringify(gripUses) !== JSON.stringify([...grips].sort())) fail(`grips rendered: ${gripUses.join(", ")} — want one each for ${grips.join(", ")}`);
  // The gesture, EVALUATED (race-check's shape): the real arrange block from
  // App.tsx, re-"rendered" after every event the way React would, under stubs
  // that count what reaches the pointer API and the settings store. A text
  // test passed with pointer capture commented out and with a settings write
  // added in an effect; these cannot.
  {
    const from = app.indexOf("  const [arrange, setArrange] = useState");
    const toLine = app.indexOf("  const resetLayout = ", from);
    const to = toLine < 0 ? -1 : app.indexOf("\n", toLine);
    if (from < 0 || to < 0) fail("App.tsx: the arrange block (`const [arrange, setArrange]` … `const resetLayout`) is gone — this check drives it");
    else {
      const body = app.slice(from, to);
      const names = ["useState", "useRef", "useEffect", "useEscape", "window", "document", "destAt", "destinations", "isCurrent", "undoOf", "resetPatch", "DEFAULTS", "GRIP_LABEL",
        "settings", "updateSettings", "setUndo", "dockShown", "selected", "lowerFits", "spaceH"];
      const ajs2 = (await transformWithEsbuild(`export function build(env) { const { ${names.join(", ")} } = env;\n${body}\n return { gripProps, closeMoveMenu, moveMenu, arrange, arranged }; }`, "arrange-block.ts", { loader: "ts", format: "esm" })).code;
      const { build } = await import("data:text/javascript;base64," + Buffer.from(ajs2).toString("base64"));
      const rects = { ".app": { left: 0, top: 0, right: 1280, bottom: 800 }, ".space-body": { left: 344, top: 50, right: 1280, bottom: 800 }, ".dock": { left: 344, top: 50, right: 704, bottom: 800 } };
      const makeApp = () => {
        const states = [], refs = [], timers = [], writes = [], captures = [], deps = [];
        let si = 0, ri = 0, ei = 0, escape = null, undo = null, pending = [];
        const settings = { places_side: "left", dock_side: "left", dock_lower: null };
        const env = {
          useState: (init) => { const i = si++; if (!(i in states)) states[i] = init; return [states[i], (v) => { states[i] = typeof v === "function" ? v(states[i]) : v; }]; },
          useRef: (init) => { const i = ri++; if (!(i in refs)) refs[i] = { current: init }; return refs[i]; },
          // Effects run after the "commit", when their deps moved — so an effect
          // that writes while the drag is live is caught by behaviour too.
          useEffect: (fn, d) => { const i = ei++; const k = JSON.stringify(d); if (deps[i] !== k) { deps[i] = k; pending.push(fn); } },
          useEscape: (fn, active) => { escape = active ? fn : null; },
          window: { setTimeout: (f) => { timers.push(f); return timers.length; } },
          document: { querySelector: (q) => (rects[q] ? { getBoundingClientRect: () => rects[q] } : null), body: { classList: { add() {}, remove() {} } } },
          destAt: A.destAt, destinations: A.destinations, isCurrent: A.isCurrent, undoOf: A.undoOf, resetPatch: A.resetPatch, DEFAULTS: S.DEFAULTS, GRIP_LABEL: A.GRIP_LABEL,
          settings, updateSettings: (p) => writes.push(p), setUndo: (u) => { undo = u; }, dockShown: true, selected: { tmux_session: { up: true } }, lowerFits: S.lowerFits, spaceH: 750,
        };
        const render = () => {
          si = 0; ri = 0; ei = 0;
          const out = build(env);
          const run = pending; pending = [];
          for (const fn of run) fn();
          return out;
        };
        const el = { setPointerCapture: (id) => captures.push(id), getBoundingClientRect: () => ({ left: 400, bottom: 70 }) };
        const ev = (x, y) => ({ button: 0, clientX: x, clientY: y, pointerId: 7, stopPropagation() {}, currentTarget: el });
        const flush = () => { while (timers.length) timers.shift()(); };
        return { render, ev, writes, captures, flush, esc: () => escape?.(), undo: () => undo };
      };
      const drag = (app_, { esc = false } = {}) => {
        app_.render().gripProps("reference").onPointerDown(app_.ev(500, 300));
        let duringMoves = -1;
        for (const x of [505, 520, 600, 700, 800, 900, 1000, 1100]) app_.render().gripProps("reference").onPointerMove(app_.ev(x, 300));
        duringMoves = app_.writes.length;
        const mid = app_.render().arrange;
        if (esc) app_.esc();
        const p = app_.render().gripProps("reference");
        p.onPointerUp(app_.ev(1100, 300));
        app_.render().gripProps("reference").onClick({ currentTarget: { getBoundingClientRect: () => ({ left: 400, bottom: 70 }) } });
        const menuAfter = app_.render().moveMenu;
        app_.flush();
        return { duringMoves, mid, menuAfter };
      };
      const wrong = [];
      let a1, r1;
      try { a1 = makeApp(); r1 = drag(a1); } catch (e) { fail(`the arrange block no longer runs under the check's stubs (${String(e).split("\n")[0]}) — a new dependency needs a stub`); }
      if (a1 && r1) {
      if (JSON.stringify(a1.captures) !== "[7]") wrong.push(`pointer capture: ${JSON.stringify(a1.captures)} (want [7]) — without it a drag across an xterm loses its moves`);
      if (r1.duringMoves !== 0) wrong.push(`${r1.duringMoves} settings write(s) DURING the drag — a live reflow resizes the agent's pty per frame`);
      if (!r1.mid || r1.mid.hover !== "ref-right") wrong.push(`mid-drag the overlay hovers ${r1.mid?.hover ?? "nothing"}, want ref-right`);
      if (a1.writes.length !== 1 || a1.writes[0]?.dock_side !== "right") wrong.push(`the drop wrote ${JSON.stringify(a1.writes)}, want one {dock_side:"right"}`);
      if (r1.menuAfter) wrong.push("the click derived from the drop opened the Move menu");
      if (!a1.undo()) wrong.push("the drop left no Undo");
      const a2 = makeApp(), r2 = drag(a2, { esc: true });
      if (a2.writes.length !== 0) wrong.push(`Esc mid-drag still wrote ${JSON.stringify(a2.writes)}`);
      if (r2.menuAfter) wrong.push("the release after Esc opened the Move menu");
      // A plain press, no movement: the click DOES open the menu.
      const a3 = makeApp();
      a3.render().gripProps("reference").onPointerDown(a3.ev(500, 300));
      a3.render().gripProps("reference").onPointerUp(a3.ev(500, 300));
      a3.render().gripProps("reference").onClick({ currentTarget: { getBoundingClientRect: () => ({ left: 400, bottom: 70 }) } });
      if (!a3.render().moveMenu || a3.writes.length) wrong.push("a press without a drag does not open the Move menu (or wrote something)");
      }
      if (wrong.length) fail(`the grip gesture (evaluated): ${wrong.join("; ")}`);
      else ok("the grip gesture, evaluated: capture on press, 0 writes over 8 moves, 1 on the drop, Esc writes nothing, menus only on a plain click");
    }
    // An effect that writes while a drag is live is the other way to build a
    // live reflow ("preview the layout on hover"). No effect keyed on the drag
    // state may write settings.
    const effects = [...app.matchAll(/useEffect\(\(\) => \{([\s\S]*?)\}, \[([^\]]*)\]\);/g)].filter((m) => /\barrange\b/.test(m[2]));
    const writing = effects.filter((m) => /update(Settings|Panels)|commitLayout/.test(m[1]));
    if (writing.length) fail(`an effect keyed on the drag writes settings: ${writing.map((m) => m[0].slice(0, 80)).join(" | ")}`);
    else ok("no effect keyed on the drag state writes settings");
  }
  const catchCss = css.match(/\.arrange-catch \{[^}]*\}/)?.[0] ?? "";
  if (!/pointer-events: none/.test(catchCss)) fail("`.arrange-catch` takes pointer events — it would sit between the captured grip and its own hit test");
  if (!bad) ok("four grips and an inert overlay");
}

console.log(bad ? `\nlayout-check: ${bad} failure(s)` : "\nlayout-check: all good");
process.exit(bad ? 1 : 0);
