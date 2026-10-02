// Checks the navigation-history RULES — app/src/navHistory.ts — by evaluating
// the real module (no DOM, no React: it is pure on purpose).
//
//   node app/scripts/navhistory-check.mjs     # exits non-zero on failure
//
// What it pins: push / same-location no-op / the 600 ms coalesce (sliding) /
// amend keeps forward / back-then-push truncates / cap / skipping dead entries
// / prune moving the current entry / normalisation (file only on Files, shell
// only on Terminal), and the labels the buttons show.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const SRC = fileURLToPath(new URL("../src/navHistory.ts", import.meta.url));
const js = (await transformWithEsbuild(fs.readFileSync(SRC, "utf8"), "navHistory.ts", {
  loader: "ts", format: "esm",
})).code;
const H = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};

const P = (slug, dock) => ({ place: { repo: "/r", slug }, ...(dock ? { dock } : {}) });
const HOME = { place: null };
const slugs = (h) => h.entries.map((e) => (e.place ? e.place.slug : "~") + (e.dock?.file ? ":" + e.dock.file.path : ""));
const W = H.COALESCE_MS;

// ── push ────────────────────────────────────────────────────────────────────
let h = H.empty();
eq("empty has no current", H.current(h), null);
h = H.record(h, HOME, 1000);
h = H.record(h, P("a"), 1000 + W);
h = H.record(h, P("b"), 1000 + 2 * W);
eq("pushes stack", [slugs(h), h.index], [["~", "a", "b"], 2]);
eq("same location twice is nothing", H.record(h, P("b"), 99_999), h);

// ── coalesce ────────────────────────────────────────────────────────────────
let c = H.record(h, P("b", { tab: "files", file: { path: "x.rs" } }), 1000 + 2 * W + 50);
eq("a change inside the window REPLACES the top", [slugs(c), c.index], [["~", "a", "b:x.rs"], 2]);
c = H.record(c, P("c"), 1000 + 2 * W + 50 + W - 1);
eq("the window slides (steady quick steps fold)", slugs(c), ["~", "a", "c"]);
c = H.record(c, P("a"), 1000 + 2 * W + 50 + W);
eq("coalescing back onto the previous entry merges with it", [slugs(c), c.index], [["~", "a"], 1]);
c = H.record(c, P("d"), 1_000_000);
eq("outside the window it pushes", slugs(c), ["~", "a", "d"]);

// ── back / forward ──────────────────────────────────────────────────────────
let b = H.step(h, -1);
eq("back moves the index", [b.index, H.current(b).place.slug], [1, "a"]);
eq("back clears the coalesce clock", b.at, 0);
eq("can go forward after back", H.canStep(b, 1), true);
eq("cannot go back past the start", H.canStep(H.step(b, -1), -1), false);
eq("step at an edge is a no-op", H.step(H.step(b, -1), -1).index, 0);
const b2 = H.record(b, P("z"), b.at + 1); // at=0 → never coalesces
eq("push after back truncates forward, never coalesces into the entry left", slugs(b2), ["~", "a", "z"]);
const am = H.record(b, P("a", { tab: "plan" }), 5, "amend");
eq("amend replaces the current entry and KEEPS forward", [slugs(am), am.index, am.entries[1].dock.tab], [["~", "a", "b"], 1, "plan"]);
const alive = (l) => !(l.place && l.place.slug === "a");
eq("back skips entries that are not alive", H.step(h, -1, alive).index, 0);
eq("reachable lists nearest first", H.reachable(h, -1), [1, 0]);
eq("canStep honours alive", H.canStep(H.goTo(h, 1), -1, (l) => l.place !== null), false);

// ── cap ─────────────────────────────────────────────────────────────────────
let big = H.empty();
for (let i = 0; i < H.CAP + 7; i++) big = H.record(big, P("p" + i), i * 10 * W);
eq("capped at CAP, oldest dropped", [big.entries.length, big.entries[0].place.slug, big.index], [H.CAP, "p7", H.CAP - 1]);

// ── prune ───────────────────────────────────────────────────────────────────
let pr = H.record(h, P("a"), 10 * W); // ~ a b a
pr = H.goTo(pr, 2); // on b
const gone = H.prune(pr, (l) => l.place?.slug === "b");
eq("prune drops entries, merges the duplicates it makes, current → survivor before", [slugs(gone), gone.index], [["~", "a"], 1]);
const keepCur = H.prune(H.goTo(pr, 3), (l) => l.place === null);
eq("prune keeps the current entry where it survives", [slugs(keepCur), keepCur.index], [["a", "b", "a"], 2]);
eq("prune with nothing dead is identity", H.prune(pr, () => false), pr);
eq("prune of everything empties", H.prune(pr, () => true), { entries: [], index: -1, at: 0 });

// ── normalisation ───────────────────────────────────────────────────────────
eq("a file off the Files tab is not part of the location",
  H.locKey(P("a", { tab: "terminal", file: { path: "x" }, shell: 2 })), H.locKey(P("a", { tab: "terminal", shell: 2 })));
eq("a shell off the Terminal tab is not part of the location",
  H.locKey(P("a", { tab: "files", shell: 2 })), H.locKey(P("a", { tab: "files" })));
eq("a different line is a different location",
  H.locKey(P("a", { tab: "files", file: { path: "x", line: 3 } })) === H.locKey(P("a", { tab: "files", file: { path: "x", line: 4 } })), false);
eq("Home drops everything else", H.locKey({ place: null, agent: "claude", dock: { tab: "files" } }), H.locKey(HOME));

// ── labels ──────────────────────────────────────────────────────────────────
eq("label: home", H.label(HOME), "Home");
eq("label: file at line", H.label(P("a", { tab: "files", file: { path: "src/App.tsx", line: 12 } })), "a · App.tsx:12");
eq("label: shell", H.label(P("a", { tab: "terminal", shell: 2 })), "a · Terminal · sh 2");
eq("label: tab, with nameOf", H.label(P("a", { tab: "docs" }), () => "Alpha"), "Alpha · Docs");

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("navhistory-check: all ok");
