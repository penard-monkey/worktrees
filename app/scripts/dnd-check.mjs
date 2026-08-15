// Checks the nav's drag-and-drop RULES — app/src/dnd.ts — against the tier
// semantics in crates/worktrees-core/src/store.rs (`reconcile`) and lib.rs
// (`LIFECYCLE_LABELS`). No DOM, no React: dnd.ts is pure on purpose.
//
//   node app/scripts/dnd-check.mjs            # exits non-zero on failure
//
// It also re-reads store.rs and asserts the two constants it mirrors still
// agree, so a change to the idle window or the sticky-label set fails HERE
// rather than as a drop that silently predicts the wrong group.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const SRC = fileURLToPath(new URL("../src/dnd.ts", import.meta.url));
const CORE = fileURLToPath(new URL("../../crates/worktrees-core/src/store.rs", import.meta.url));
const APP_RS = fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url));

const js = (await transformWithEsbuild(fs.readFileSync(SRC, "utf8"), "dnd.ts", {
  loader: "ts", format: "esm",
})).code;
const D = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
const ok = (name, cond) => eq(name, !!cond, true);

const NOW = 1_700_000_000;
const place = (over = {}) => ({
  slug: "feature-x", is_main: false, tmux_session: { up: false }, declared: {}, ...over,
});

// ── the constants this module mirrors ───────────────────────────────────────
const core = fs.readFileSync(CORE, "utf8");
const idle = /IDLE_WINDOW_SECS: i64 = ([^;]+);/.exec(core);
// `7 * 24 * 3600` — a product of integer literals, multiplied out by hand
// rather than eval'd (the value is read from a source file).
const product = idle[1].replace(/_/g, "").split("*").reduce((a, s) => a * Number(s.trim()), 1);
eq("IDLE_WINDOW_SECS mirrors store.rs", D.IDLE_WINDOW_SECS, product);
const sticky = [...core.matchAll(/Some\("(\w+)"\) => return "\1"/g)].map((m) => m[1]).sort();
eq("sticky labels mirror reconcile()", sticky, ["abandoned", "archived", "saved"]);
const labels = /LIFECYCLE_LABELS: \[&str; \d+\] = \[([^\]]+)\]/.exec(fs.readFileSync(APP_RS, "utf8"));
eq("LIFECYCLE_LABELS mirrors lib.rs",
  [...D.LIFECYCLE_LABELS].sort(),
  labels[1].split(",").map((s) => s.trim().replace(/"/g, "")).filter(Boolean).sort());

// ── predictTier == reconcile + bucketOf ─────────────────────────────────────
eq("pin outranks everything",
  D.predictTier(place({ declared: { lifecycle: "archived" } }), { pinned: true }, NOW), "pinned");
eq("sticky label beats live tmux",
  D.predictTier(place({ tmux_session: { up: true }, declared: { lifecycle: "archived" } }), {}, NOW),
  "archived");
eq("tmux up reads active",
  D.predictTier(place({ tmux_session: { up: true } }), {}, NOW), "active");
eq("declared closed is NOT sticky — falls through to derived",
  D.predictTier(place({ tmux_session: { up: true } }), { lifecycle: "closed" }, NOW), "active");
eq("recently opened reads idle",
  D.predictTier(place({ declared: { last_opened_epoch: NOW - 3600 } }), {}, NOW), "idle");
eq("opened past the window reads closed",
  D.predictTier(place({ declared: { last_opened_epoch: NOW - D.IDLE_WINDOW_SECS - 1 } }), {}, NOW),
  "closed");
eq("never opened reads closed", D.predictTier(place(), {}, NOW), "closed");
eq("clearing the label re-derives",
  D.predictTier(place({ declared: { lifecycle: "archived", last_opened_epoch: NOW - 60 } }),
    { lifecycle: null }, NOW), "idle");

// ── dropIntent ──────────────────────────────────────────────────────────────
ok("main worktree rejects", D.dropIntent(place({ is_main: true }), "pinned", NOW, "idle").ok === false);
ok("Active rejects (derived from tmux)", D.dropIntent(place(), "active", NOW, "closed").ok === false);
ok("Active's rejection explains itself",
  /tmux/.test(D.dropIntent(place(), "active", NOW, "closed").hint));
eq("drop on Pinned pins, leaves the label alone",
  D.dropIntent(place({ declared: { lifecycle: "saved" } }), "pinned", NOW, "saved").patch, { pinned: true });
eq("drop on Idle unpins AND clears the label",
  D.dropIntent(place({ declared: { pinned: true, lifecycle: "archived" } }), "idle", NOW, "pinned").patch,
  { pinned: false, lifecycle: null });
eq("drop on a dormant tier also unpins — else pinned wins and nothing moves",
  D.dropIntent(place({ declared: { pinned: true } }), "archived", NOW, "pinned").patch,
  { pinned: false, lifecycle: "archived" });
eq("a pinned row dropped on Archived really lands in Archived",
  D.dropIntent(place({ declared: { pinned: true } }), "archived", NOW, "pinned").lands, "archived");
eq("dropping a LIVE place on Closed still reads Active",
  D.dropIntent(place({ tmux_session: { up: true } }), "closed", NOW, "active").lands, "active");
eq("dropping a cold, never-opened place on Idle lands in Closed",
  D.dropIntent(place({ declared: { pinned: true } }), "idle", NOW, "pinned").lands, "closed");
ok("…and that mismatch is explained",
  /Idle is derived/.test(D.landingNote("idle", "closed")));
ok("a live session's mismatch names tmux",
  /tmux session is running/.test(D.landingNote("closed", "active")));
eq("no note when the row lands where it was dropped", D.landingNote("archived", "archived"), null);
// A drop inside a row's OWN group is a re-order, never a write — which is the
// only way the derived groups stay reorderable at all.
ok("same-group drop is allowed even for Active",
  D.dropIntent(place({ tmux_session: { up: true } }), "active", NOW, "active").ok);
eq("…and writes nothing",
  D.dropIntent(place({ tmux_session: { up: true } }), "active", NOW, "active").patch, {});
ok("same-group drop is allowed for Idle too",
  D.dropIntent(place({ declared: { last_opened_epoch: NOW - 60 } }), "idle", NOW, "idle").ok);
ok("main is still refused even inside its own group",
  D.dropIntent(place({ is_main: true }), "pinned", NOW, "pinned").ok === false);
// App.tsx `applyTier` writes a predictTier answer straight into the row's
// `lifecycle_effective` whenever a drop patches the label (the optimistic
// badge). That is sound only while every lifecycle-writing plan lands on a
// value `reconcile` can produce — "pinned" is bucketOf's pseudo-tier, not a
// badge, and a plan that leaked it would paint a row wearing a badge the
// backend can never confirm. The pin drop stays out of badge territory by
// carrying no lifecycle key at all (its patch is exactly `{ pinned: true }`,
// checked above). Exercised from the worst case: a pinned, live, recently
// opened, sticky-labelled place, so every branch of predictTier is in play.
for (const t of ["pinned", "idle", "saved", "closed", "archived", "abandoned"]) {
  const plan = D.dropIntent(
    place({
      tmux_session: { up: true },
      declared: { pinned: true, lifecycle: "saved", last_opened_epoch: NOW - 60 },
    }),
    t, NOW, "pinned");
  ok(`a lifecycle-writing plan never lands on "pinned" (→ ${t})`,
    plan.ok && (plan.patch.lifecycle === undefined || plan.lands !== "pinned"));
}

// ── landing position ────────────────────────────────────────────────────────
eq("alpha asc: middle", D.alphaIndex(["alpha", "gamma"], "beta", false), 1);
eq("alpha asc: first", D.alphaIndex(["beta", "gamma"], "alpha", false), 0);
eq("alpha asc: last", D.alphaIndex(["alpha", "beta"], "zeta", false), 2);
eq("alpha desc mirrors", D.alphaIndex(["zeta", "gamma"], "yak", true), 1);
eq("alpha into an empty group", D.alphaIndex([], "solo", false), 0);
eq("recent desc: newest first", D.recentIndex([300, 100], 200, false), 1);
eq("recent asc flips", D.recentIndex([100, 300], 200, true), 1);
// Equal keys insert AFTER their run — where a stable sort puts a newcomer, so
// the gap's promised slot matches what sortPlaces will actually produce.
eq("equal keys insert after the run of equals",
  D.sortedIndex(["a", "b", "b", "c"], "b", (x, y) => x.localeCompare(y)), 3);
eq("pointer above the first row", D.pointerIndex([{ top: 0, height: 20 }], 4), 0);
eq("pointer past a row's midpoint", D.pointerIndex([{ top: 0, height: 20 }], 16), 1);
eq("pointer between rows",
  D.pointerIndex([{ top: 0, height: 20 }, { top: 20, height: 20 }], 24), 1);
eq("pointer below every row",
  D.pointerIndex([{ top: 0, height: 20 }, { top: 20, height: 20 }], 100), 2);

// The gap-compensation that keeps the gap from chasing the pointer. Layout:
// rows of 20px, a 20px gap open at index 1, so live tops are 0, 40 (row 1 was
// pushed down by the gap sitting at 20).
const GAP_TOP = 20, GAP_H = 20;
const nat = (t) => D.naturalTop(t, GAP_TOP, GAP_H);
eq("a row above the gap is unmoved", nat(0), 0);
eq("a row below the gap comes back up", nat(40), 20);
eq("a pointer inside the gap clamps to the gap's top", nat(30), 20);
eq("a pointer at the gap's very bottom still clamps", nat(39), 20);
// …and with that compensation the slot the pointer names is the slot the gap
// is ALREADY in — the fixed point that makes it stop moving.
eq("resting in the gap re-computes the same slot",
  D.pointerIndex([{ top: nat(0), height: 20 }, { top: nat(40), height: 20 }], nat(30)), 1);
eq("moving below the next row advances one slot",
  D.pointerIndex([{ top: nat(0), height: 20 }, { top: nat(40), height: 20 }], nat(55)), 2);

// ── manual order ────────────────────────────────────────────────────────────
eq("normalize drops slugs that no longer exist",
  D.normalizeOrder(["gone", "a"], ["a", "b"]), ["a", "b"]);
eq("normalize de-dupes", D.normalizeOrder(["a", "a", "b"], ["a", "b"]), ["a", "b"]);
eq("normalize appends unseen slugs in snapshot order",
  D.normalizeOrder(["c"], ["a", "b", "c"]), ["c", "a", "b"]);
eq("splice moves before the target", D.spliceOrder([], ["a", "b", "c"], "c", "b"), ["a", "c", "b"]);
eq("splice to the end", D.spliceOrder([], ["a", "b", "c"], "a", null), ["b", "c", "a"]);
eq("splice is a no-op onto itself", D.spliceOrder([], ["a", "b"], "a", "a"), ["a", "b"]);
// Cross-tier drop: the flat array only has to get the WITHIN-group order right.
eq("moving into another group orders it inside that group",
  D.spliceOrder(["pin1", "pin2", "idle1"], ["pin1", "pin2", "idle1"], "idle1", "pin2"),
  ["pin1", "idle1", "pin2"]);
eq("project move by root",
  D.moveBefore(["/a", "/b", "/c"], "/c", "/a"), ["/c", "/a", "/b"]);
eq("project move to the end", D.moveBefore(["/a", "/b"], "/a", null), ["/b", "/a"]);

console.log(failed ? `\n${failed} check(s) failed` : "all dnd checks passed");
process.exit(failed ? 1 : 0);
