#!/usr/bin/env node
// hubs-check — the hub rules the app applies before the backend sees a sync
// (`app/src/hubs.ts`), run against the real source.
//
// The one that matters most: a CHOSEN hub that is unplugged is never swapped for
// whatever other drive is connected. The backend's chain refuses that swap; the
// app picks a hub id BEFORE it asks, so a frontend default that falls back to
// "the auto-detected drive" silently pushes to the wrong one (review of #587).
// With anything chosen, the default is chosen ∩ connected — possibly empty, in
// which case the app sends `hub: null` and the backend's named error is shown.
//
// Also: a pull defaults to the NEWEST connected copy; a batch pull leaves out a
// project that has no root here (an import is the single modal's act, where its
// destination is shown).

import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let bad = 0;
const fail = (m) => { console.error(`not ok ${m}`); bad++; };
const eq = (got, want, what) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
};
const load = async (src, name) => {
  const js = (await transformWithEsbuild(src, name, { loader: "ts", format: "esm" })).code;
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
};

const H = await load(read("../src/hubs.ts"), "hubs.ts");
const hub = (id, o = {}) => ({
  id, name: id, kind: "removable", path: `/Volumes/${id}`, connected: true, chosen: null, configured: false,
  auto: false, last_seen: 1, free_bytes: null, total_bytes: null, projects: [], stale: false, ...o,
});
const dash = (hubs, local = []) => ({ hubs, local, default_error: null });

// ── push defaults ────────────────────────────────────────────────────────────
// The swap: Extreme SSD chosen and unplugged, SanDisk plugged in (auto).
eq(H.defaultPushHubs(dash([hub("SSD", { chosen: 0, connected: false, path: null }), hub("SanDisk", { auto: true })])), [],
  "chosen hub unplugged + another drive connected → no default (never the other drive)");
eq(H.defaultPushHubs(dash([hub("SSD", { chosen: 0, connected: false, path: null }), hub("Only")])), [],
  "chosen hub unplugged + the only connected drive → still no default");
eq(H.defaultPushHubs(dash([hub("A", { chosen: 1 }), hub("B", { chosen: 0 }), hub("C", { auto: true })])), ["B", "A"],
  "chosen and connected → those, in chosen order, and not the auto drive");
eq(H.defaultPushHubs(dash([hub("A", { chosen: 0 }), hub("B", { chosen: 1, connected: false, path: null })])), ["A"],
  "one chosen hub unplugged → the connected chosen ones only");
eq(H.defaultPushHubs(dash([hub("A"), hub("B", { auto: true })])), ["B"], "nothing chosen → the auto-detected drive");
eq(H.defaultPushHubs(dash([hub("S", { kind: "network" })])), ["S"], "nothing chosen, one hub connected → it");
eq(H.defaultPushHubs(dash([hub("A"), hub("B")])), [], "nothing chosen, two unchosen and no auto → no guess");

// ── pull default: newest connected copy ─────────────────────────────────────
const p = (name, at) => ({ name, local_root: `/r/${name}`, pushed_at: at, host: "h", here: null });
const two = dash([
  hub("Old", { chosen: 0, projects: [p("x", 10)] }),
  hub("New", { projects: [p("x", 20)] }),
  hub("Gone", { connected: false, path: null, projects: [p("x", 99)] }),
]);
eq(H.newestHub(two, "x"), "New", "pull defaults to the newest CONNECTED copy, not the primary or an unplugged one");

// ── batch pull leaves out hub-only projects ─────────────────────────────────
eq(H.batchJobs("pull", [{ name: "here", repo: "/r/here" }, { name: "away", repo: null }], [], { here: "A", away: "A" }),
  [{ repo: "/r/here", name: null, hub: "A" }],
  "a batch pull never imports (a hub-only project's destination is shown only in the single import modal)");

if (bad) { console.error(`\nhubs-check: ${bad} failure(s)`); process.exit(1); }
console.log("hubs-check: ok — chosen-but-unplugged never swapped, newest copy pulled, batch pull never imports");
