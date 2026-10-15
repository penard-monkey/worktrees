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
  if (!/<aside className="dock" style=\{\{ order: dockLeft \? -1 : 0/.test(app)) {
    fail("App.tsx: `.dock`'s order is not `dockLeft ? -1 : 0` — the dock no longer follows dock_side");
  } else ok("the dock's position inside .space follows dock_side");
  if (!/const dockLeft = settings\.dock_side === "left";/.test(app) || !/const merged = railsMerged\(settings\);/.test(app)) {
    fail("App.tsx: `dockLeft` / `merged` are not derived from settings.dock_side / railsMerged(settings)");
  }
  if (!/dockLeft \? ev\.clientX - startX : startX - ev\.clientX/.test(app)) {
    fail("App.tsx: the dock resizer's sign is not keyed on dockLeft — pulling it open would shrink it on one side");
  } else ok("the dock resizer's sign follows dock_side");
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
      const before = app.slice(places[0], inPlaces[0]);
      const gate = before.slice(before.lastIndexOf("{") + 1);
      if (!/^merged &&/.test(gate.trim())) fail(`${name} in the Places rail is gated on \`${gate.trim().slice(0, 60)}\`, not \`merged && …\``);
    }
    // Merged, the dock's buttons sit in the auto-hide nav's hover edge. They
    // must be wrapped in `.rail-group` and the rail must skip that band, or
    // reaching for Terminal slides Places over the reference panel.
    const placesRail = app.slice(places[0], places[1]);
    const head = app.slice(places[0], app.indexOf(">\n", places[0] + 20));
    if (!/<div className="rail-group">\s*<DockRailGroup /.test(placesRail)) fail("the merged DockRailGroup is not wrapped in `.rail-group` — the hover edge cannot tell it apart");
    else if (!/closest\("\.rail-group"\)/.test(head) || /onPointerEnter=\{\(\) => navPointer\("rail", true\)\}/.test(head)) {
      fail("the Places rail reveals the nav on ANY pointer entry — hovering a merged dock button would slide Places over the reference panel");
    } else ok("the merged dock group is outside the nav's hover edge");
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

console.log(bad ? `\nlayout-check: ${bad} failure(s)` : "\nlayout-check: all good");
process.exit(bad ? 1 : 0);
