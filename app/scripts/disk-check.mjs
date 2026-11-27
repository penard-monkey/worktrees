// Guards the Disk view's pure rules (app/src/diskmodel.ts).
//
//   node app/scripts/disk-check.mjs        # exits non-zero on failure
//
// WHY THIS EXISTS. The treemap is a picture of numbers, and a wrong picture
// looks as plausible as a right one: a layout that drops an item, overlaps
// two, or leaves area unused still draws coloured boxes. And the cleanup
// sheet's defaults decide what a single click deletes. So this pins:
//
//   1. THE LAYOUT. squarify gives every positive value one rectangle, inside
//      the box, none overlapping, each with area proportional to its value,
//      the whole box used.
//   2. THE TREES. A place's leaves add up to what it holds, with or without
//      its shared bytes; the engine's pruned tree keeps every byte.
//   3. THE CLEANUP. Only build output is offered (never dependencies, git or
//      source), and a place is ticked by default only when no agent works in
//      it and it has been idle IDLE_DAYS.
//
// Same slice-the-real-source shape as offers-check.mjs: it evaluates
// `diskmodel.ts` itself, so it tests the edit and not a paraphrase.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// vite's esbuild re-export — see race-check.mjs: bare "esbuild" does not resolve
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

const src = read("../src/diskmodel.ts");
if (/^import /m.test(src)) fail("diskmodel.ts gained an import: this check loads it as a data: module, which cannot resolve one");
const js = (await transformWithEsbuild(src, "disk-check.ts", { loader: "ts", format: "esm" })).code;
const m = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);

// ── 1. the layout ──────────────────────────────────────────────────────────
{
  const cases = [
    [[6, 6, 4, 3, 2, 2, 1], 600, 400],
    [[100], 300, 200],
    [[50, 0, 25, 25], 200, 500],
    [[9e9, 3e8, 2e8, 1e6, 5e5], 1200, 700],
    [Array.from({ length: 60 }, (_, i) => 60 - i), 800, 300],
  ];
  let fine = true;
  for (const [vals, W, H] of cases) {
    const r = m.squarify(vals, 10, 20, W, H);
    const pos = vals.map((v, i) => [v, i]).filter(([v]) => v > 0);
    const total = pos.reduce((a, [v]) => a + v, 0);
    if (r.length !== pos.length) { fail(`squarify ${vals.length} values → ${r.length} rects, want ${pos.length}`); fine = false; continue; }
    if (new Set(r.map((x) => x[0])).size !== r.length) { fail("squarify gave an item two rectangles"); fine = false; }
    let area = 0;
    for (const [i, x, y, w, h] of r) {
      area += w * h;
      const want = (vals[i] / total) * W * H;
      if (Math.abs(w * h - want) > want * 1e-6 + 1e-6) { fail(`item ${i}: area ${w * h} for value ${vals[i]}, want ${want}`); fine = false; }
      if (x < 10 - 1e-6 || y < 20 - 1e-6 || x + w > 10 + W + 1e-6 || y + h > 20 + H + 1e-6) { fail(`item ${i} leaves the box`); fine = false; }
    }
    if (Math.abs(area - W * H) > 1e-3) { fail(`rects cover ${area}, box is ${W * H}`); fine = false; }
    for (let a = 0; a < r.length; a++) for (let b = a + 1; b < r.length; b++) {
      const [, ax, ay, aw, ah] = r[a], [, bx, by, bw, bh] = r[b];
      const ox = Math.min(ax + aw, bx + bw) - Math.max(ax, bx), oy = Math.min(ay + ah, by + bh) - Math.max(ay, by);
      if (ox > 1e-6 && oy > 1e-6) { fail(`rects ${r[a][0]} and ${r[b][0]} overlap`); fine = false; }
    }
  }
  if (m.squarify([], 0, 0, 10, 10).length || m.squarify([0, 0], 0, 0, 10, 10).length || m.squarify([1], 0, 0, 0, 10).length) {
    fail("nothing to lay out must give no rectangles"); fine = false;
  }
  if (fine) ok("squarify: one rect per positive value, proportional, inside, no overlap, box covered");
}

// ── 2. the trees ───────────────────────────────────────────────────────────
{
  const u = { slug: "a", path: "/p/a", is_main: false, own: { build: 5, deps: 3, source: 2, git: 0 }, shared: 7, alloc: 17, files: 1, errors: 0, cleanable: [] };
  const onlyHere = m.tmValue(m.placeTM("/p", u, false));
  const onDisk = m.tmValue(m.placeTM("/p", u, true));
  if (onlyHere !== 10) fail(`a place's "only here" leaves sum to ${onlyHere}, want 10`);
  else if (onDisk !== 17) fail(`a place's "on disk" leaves sum to ${onDisk}, want 17`);
  else ok("a place's leaves add up to its own bytes, and with the shared leaf to everything it holds");
  if (m.placeTM("/p", u, true).kids.filter((k) => k.shared).length !== 1) fail("the shared bytes must be one hatched leaf");

  const tree = {
    n: "", own: 90, shared: 10, files: 9, c: "source", dir: true, id: "1:1",
    k: [
      { n: "target", p: "target", own: 60, shared: 0, files: 3, c: "build", dir: true, id: "1:2",
        k: [{ n: "debug", p: "target/debug", own: 50, shared: 0, files: 2, c: "build", dir: true, id: "1:3", k: [{ n: "", own: 50, shared: 0, files: 2, c: "build" }] },
            { n: "", own: 10, shared: 0, files: 1, c: "build" }] },
      { n: "big.bin", p: "big.bin", own: 20, shared: 0, files: 1, c: "source", id: "1:4" },
      { n: "", own: 10, shared: 0, files: 4, c: "source" },
      { n: "", own: 0, shared: 10, files: 1, c: "deps" },
    ],
  };
  const a = m.tmValue(m.treeTM(tree, true)), b = m.tmValue(m.treeTM(tree, false));
  if (a !== 100 || b !== 90) fail(`the engine's tree sums to ${a}/${b}, want 100 with shared and 90 without`);
  else ok("the engine's pruned tree keeps every byte, shared ones only when asked");
  const t = m.treeTM(tree, true);
  const debug = t.kids.find((k) => k.label === "target").kids.find((k) => k.label === "debug");
  if (debug?.ref?.path !== "target/debug" || debug?.ref?.id !== "1:3") fail("a folder keeps its path and its scan identity, which a delete is checked against");
  else ok("a folder keeps its path and its scan identity");
  if (t.kids.some((k) => !k.label && k.ref?.path)) fail("a folded aggregate must carry no path: nothing deletes 'small files'");
}

// ── 3. the cleanup ─────────────────────────────────────────────────────────
{
  const now = 2_000_000_000;
  const day = 86400;
  const c = (path, cat, own) => ({ path, cat, own, shared: 0, files: 1, id: "1:" + path.length });
  const usage = {
    "/r": {
      root: "/r", scanned_epoch: now, scan_ms: 1, unique: 0, volume: { free: 0, total: 0, snapshots: null },
      places: [
        { slug: "(main)", path: "/r", is_main: true, own: { build: 0, deps: 0, source: 0, git: 0 }, shared: 0, alloc: 0, files: 0, errors: 0, cleanable: [c("target", "build", 5e9), c("node_modules", "deps", 9e9)] },
        { slug: "old", path: "/r/.worktrees/old", is_main: false, own: { build: 0, deps: 0, source: 0, git: 0 }, shared: 0, alloc: 0, files: 0, errors: 0, cleanable: [c("target", "build", 2e9), c(".next", "build", 1e9)] },
        { slug: "busy", path: "/r/.worktrees/busy", is_main: false, own: { build: 0, deps: 0, source: 0, git: 0 }, shared: 0, alloc: 0, files: 0, errors: 0, cleanable: [c("target", "build", 8e9)] },
        { slug: "tiny", path: "/r/.worktrees/tiny", is_main: false, own: { build: 0, deps: 0, source: 0, git: 0 }, shared: 0, alloc: 0, files: 0, errors: 0, cleanable: [c("dist", "build", 5e7)] },
        { slug: "deps-only", path: "/r/.worktrees/deps-only", is_main: false, own: { build: 0, deps: 0, source: 0, git: 0 }, shared: 0, alloc: 0, files: 0, errors: 0, cleanable: [c("node_modules", "deps", 9e9)] },
      ],
    },
  };
  const info = (_r, slug) => ({ last: slug === "old" ? now - 30 * day : slug === "busy" ? now - 60 * day : now - 1 * day, busy: slug === "busy" });
  const rows = m.bulkRows(usage, { "/r": "r" }, info);
  const slugs = rows.map((r) => r.slug).join(",");
  if (slugs !== "busy,(main),old") fail(`cleanup rows ${slugs}: want busy,(main),old (largest first; no deps-only, nothing under 100 MB)`);
  else ok("cleanup offers places with build output, largest first, nothing under 100 MB");
  if (rows.some((r) => r.items.some((i) => i.cat !== "build"))) fail("cleanup offered a dependency folder: a reinstall is slower and can change versions");
  else ok("cleanup offers build output only, never dependencies");
  const old = rows.find((r) => r.slug === "old");
  if (old?.frees !== 3e9) fail(`a row frees ${old?.frees}, want the sum of its build folders (3e9)`);
  const ticked = rows.filter((r) => m.bulkDefault(r, now)).map((r) => r.slug).join(",");
  if (ticked !== "old") fail(`ticked by default: ${ticked}; want only "old" (busy has an agent working, main committed yesterday)`);
  else ok(`ticked by default only when idle ${m.IDLE_DAYS} days and no agent is working`);
  if (m.bulkDefault({ last: null, busy: false }, now)) fail("a place with no commit date must not be ticked by default");
}

// ── 4. units ───────────────────────────────────────────────────────────────
{
  const want = [[2e12, "2.0 TB"], [741e9, "741 GB"], [17.1e9, "17.1 GB"], [880e6, "880 MB"], [5e5, "<1 MB"], [0, "0 MB"]];
  const wrong = want.filter(([v, s]) => m.gb(v) !== s);
  if (wrong.length) fail(`gb: ${wrong.map(([v, s]) => `${v} → ${m.gb(v)}, want ${s}`).join("; ")}`);
  else ok("sizes in decimal units, as Finder says them");
}

if (bad) { console.error(`\n${bad} failure(s)`); process.exit(1); }
// ── 4. the pick ────────────────────────────────────────────────────────────
// A treemap pick is kept as a PATH (it is part of the history location), so
// Back must find the node again in whatever tree is drawn, or say it is gone.
{
  const t = m.treeTM({ n: "", own: 0, shared: 0, files: 0, c: "source", dir: true, k: [
    { n: "target", p: "target", own: 0, shared: 0, files: 0, c: "build", dir: true, k: [
      { n: "debug", p: "target/debug", own: 5, shared: 0, files: 1, c: "build" },
      { n: "", own: 2, shared: 0, files: 1, c: "build" },
    ] },
  ] }, false);
  const f = m.findPath(t, "target/debug");
  if (f?.ref?.path !== "target/debug") fail(`findPath: the picked folder is found, got ${f?.ref?.path}`);
  else ok("findPath: a picked path is found again in the tree");
  if (m.findPath(t, "target/release") !== null) fail("findPath: a path that is gone answers null");
  else ok("findPath: a path that is gone (deleted, rescanned away) answers null");
}

console.log("\nall disk checks passed");
