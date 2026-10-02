// Checks App.tsx's navigation-history WIRING — the parts navhistory-check.mjs
// cannot see because they live in the component, not in navHistory.ts.
//
// Same shape as race-check.mjs: the REAL source is sliced out of App.tsx
// between stable markers and evaluated with stubs, so it tests the edit, not a
// paraphrase of it.
//
//   node app/scripts/navwire-check.mjs [path/to/App.tsx]   # non-zero on failure
//
// 1. The observed location (`frontHere` … `navLoc`). Waiting for the front
//    shell is right only while the tab strip is MOUNTED and restoring. A place
//    whose dock is closed on its Terminal tab has no strip at all, so waiting
//    there meant the place was never recorded — and since `updatePanels`
//    spreads the dock state over the globals, neither was any place visited
//    after it.
// 2. The parked apply (`const loc = navPending;` effect). Its `file_readable`
//    answer is asynchronous; an answer for an entry history has already moved
//    PAST must be dropped, or a stale stat opens one place's file inside
//    another and the observer pushes it, truncating forward history.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const APP = process.argv[2] || fileURLToPath(new URL("../src/App.tsx", import.meta.url));
const lines = fs.readFileSync(APP, "utf8").split("\n");
let failed = 0;
const fail = (m) => { failed++; console.log("not ok — " + m); };
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g !== w) fail(`${name}\n     got: ${g}\n    want: ${w}`);
};
const load = async (src, name) => {
  const js = (await transformWithEsbuild(src, name, { loader: "ts", format: "esm" })).code;
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
};
const at = (needle, from = 0) => lines.findIndex((l, i) => i >= from && l.includes(needle));

// ── 1. the observed location ────────────────────────────────────────────────
const a = at("const frontHere =");
const b = at("const navKey =", a);
if (a < 0 || b < 0) { console.log("not ok — App.tsx: `const frontHere =` … `const navKey =` markers are gone"); process.exit(1); }
const LOC = lines.slice(a, b).join("\n");
const ENV1 = ["sel", "selected", "eff", "dockShown", "dockFile", "dockAt", "termFront", "settings", "placeKey", "navSeenFront"];
const { build: buildLoc } = await load(`
export function build(env: any) {
  const { ${ENV1.join(", ")} } = env;
${LOC}
  return navLoc;
}`, "navloc.ts");

const placeKey = (r, s) => `${r}|${s}`;
const P1 = { repo: "/r", slug: "p1" };
const navLoc = (o) => buildLoc({
  sel: P1, selected: {}, dockFile: null, dockAt: null, settings: {}, placeKey, termFront: null,
  navSeenFront: { current: {} },
  ...o,
  eff: { dock_open: o.open, dock_tab: o.tab },
  dockShown: o.open,
});
const front = (id) => ({ key: placeKey(P1.repo, P1.slug), id });

eq("dock CLOSED on Terminal (no strip mounted) is still a location",
  navLoc({ open: false, tab: "terminal", termFront: front(undefined) })?.place, P1);
eq("…and records the remembered front shell, so opening the dock (⌘J) is not a new entry",
  navLoc({ open: false, tab: "terminal", termFront: front(undefined), settings: { term_tab_active: { "/r|p1": 2 } } })?.dock,
  { tab: "terminal", shell: 2 });
eq("…the same location the open dock reports once its strip has restored onto that tab",
  navLoc({ open: true, tab: "terminal", termFront: front(2) })?.dock, { tab: "terminal", shell: 2 });
// ⌘J is layout, not a step. The strip's own restore never writes the
// remembered tab (only a pick does), so on a place where nothing was picked
// the closed dock must fall back to what the strip last SHOWED.
eq("⌘J hiding a strip that restored onto sh 1 (never picked) keeps the location",
  navLoc({ open: false, tab: "terminal", termFront: front(undefined), navSeenFront: { current: { "/r|p1": 1 } } })?.dock,
  navLoc({ open: true, tab: "terminal", termFront: front(1) })?.dock);
eq("a stale front from ANOTHER place never leaks into a closed dock's location",
  navLoc({ open: false, tab: "terminal", termFront: { key: "/r|other", id: 5 } })?.dock, { tab: "terminal" });
eq("dock OPEN on Terminal while the strip restores: not known yet, so not recorded",
  navLoc({ open: true, tab: "terminal", termFront: front(undefined) }), null);
eq("dock open on Terminal with the session down (no tabs) is a location without a shell",
  navLoc({ open: true, tab: "terminal", termFront: front(null) })?.dock, { tab: "terminal" });
eq("dock closed on Files is a location",
  navLoc({ open: false, tab: "files" })?.dock, { tab: "files" });
eq("Home", navLoc({ sel: null, open: false, tab: "files" }), { place: null });

// ── 2. the parked apply's async file landing ────────────────────────────────
const e0 = at("const loc = navPending;");
const e1 = at("}, [navPending, sel]);", e0);
if (e0 < 1 || e1 < 0 || !lines[e0 - 1].includes("useEffect(() => {")) {
  console.log("not ok — App.tsx: the parked-apply effect (`const loc = navPending;` … `}, [navPending, sel]);`) is gone");
  process.exit(1);
}
const EFFECT = lines.slice(e0 - 1, e1 + 1).join("\n");
const ENV2 = ["useEffect", "navPending", "sel", "selRef", "setNavPending", "eff", "dockFile", "dockAt", "openDockFile",
  "invoke", "setNotice", "setDockFile", "setDockAt", "updatePanels", "settings", "placeKey", "frontHere",
  "setTermTab", "setNavShellGoto", "navShellSeq", "navApplyGen"];
const { build: buildApply } = await load(`
export function build(env: any) {
  const { ${ENV2.join(", ")} } = env;
${EFFECT}
}`, "navapply.ts");

/** One app: refs persist across applies, as they do across renders. */
function app() {
  const opened = [], stats = [];
  const refs = { selRef: { current: null }, navShellSeq: { current: 0 }, navApplyGen: { current: 0 } };
  /** Run the effect as React would, for this entry with the app in this state. */
  const apply = (entry, state) => {
    refs.selRef.current = entry.place; // the selection the entry named has landed
    let fx;
    buildApply({
      ...refs,
      useEffect: (fn) => { fx = fn; },
      navPending: entry, sel: entry.place, setNavPending: () => {},
      eff: { dock_open: true, dock_tab: "files" }, dockFile: null, dockAt: null, frontHere: undefined,
      settings: {}, placeKey,
      invoke: (cmd, args) => new Promise((res) => stats.push({ path: args.path, res })),
      openDockFile: (path) => opened.push(path),
      setNotice: () => {}, setDockFile: () => {}, setDockAt: () => {}, updatePanels: () => {},
      setTermTab: () => {}, setNavShellGoto: () => {},
      ...state,
    });
    fx();
  };
  return { apply, opened, stats, refs };
}
const fileAt = (place, path) => ({ place, dock: { tab: "files", file: { path } } });
const P2 = { repo: "/r", slug: "p2" };
const tick = () => new Promise((r) => setTimeout(r, 0));

{ // ⌘[ ⌘[ fast: P2's slow stat answers after history has landed on P1
  const t = app();
  t.apply(fileAt(P2, "/p2/Cargo.toml"));
  t.apply(fileAt(P1, "/p1/README.md"));
  t.stats[1].res(true); await tick();
  t.stats[0].res(true); await tick();
  eq("two Backs, the first stat answering LAST: only the file of the entry landed on opens",
    t.opened, ["/p1/README.md"]);
}
{ // the same race inside one place: two entries, two files
  const t = app();
  t.apply(fileAt(P1, "/p1/a.md"));
  t.apply(fileAt(P1, "/p1/b.md"));
  t.stats[1].res(true); await tick();
  t.stats[0].res(true); await tick();
  eq("two Backs within ONE place: the stale answer does not replace the newer file", t.opened, ["/p1/b.md"]);
}
{ // you clicked somewhere else while the stat was out
  const t = app();
  t.apply(fileAt(P2, "/p2/Cargo.toml"));
  t.refs.selRef.current = P1;
  t.stats[0].res(true); await tick();
  eq("a selection that moved away while the stat was out drops the answer", t.opened, []);
}
{ // and the plain case still works
  const t = app();
  t.apply(fileAt(P2, "/p2/Cargo.toml"));
  t.stats[0].res(true); await tick();
  eq("an answer for the entry still being shown opens its file", t.opened, ["/p2/Cargo.toml"]);
}

if (failed) { console.log(`\nnavwire-check: ${failed} failed`); process.exit(1); }
console.log(`navwire-check: ok — ${LOC.split("\n").length}L location + ${EFFECT.split("\n").length}L apply effect of App.tsx driven`);
