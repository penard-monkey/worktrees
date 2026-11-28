// The return rule for Where you left off, on a scripted clock.
//
//   node app/scripts/returning-check.mjs   # from the repo root
//
// Override RETURNING_SRC to exercise a mutation without touching the tree.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = process.env.RETURNING_SRC ?? here("../src/returning.ts");
const src = fs.readFileSync(SRC, "utf8");
let failed = 0;
const check = (cond, message) => {
  console.log(`${cond ? "ok" : "not ok"} — ${message}`);
  if (!cond) failed++;
};
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

check(!/^\s*import\s(?!type\b)/m.test(src), "returning.ts has no runtime import");
check(!/\b(?:Date|performance)\s*\.\s*now\b|\bnew\s+Date\b/.test(src), "the module reads neither the clock nor the local date itself");
const js = (await transformWithEsbuild(src, path.basename(SRC), { loader: "ts", format: "esm" })).code;
const R = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

const HOUR = 60 * 60 * 1000;
const BASE = Date.parse("2026-11-27T08:00:00Z");
const TODAY = "2026-11-27";
const YESTERDAY = "2026-11-26";

function machine({ lastActive = BASE, shownDay = YESTERDAY, now = BASE, lastPersisted = lastActive } = {}) {
  let state = { lastActive, shownDay, lastPersisted };
  return {
    advance(ms) { now += ms; },
    send(event, today = TODAY) {
      const step = R.reduceReturning(state, event, now, today);
      state = step.state;
      return step;
    },
    mark(today = TODAY) { state = R.markShown(state, today); },
    state: () => state,
    now: () => now,
  };
}

check(R.RETURN_AFTER_MS === 6 * HOUR, `the away threshold is one constant: six hours (${R.RETURN_AFTER_MS} ms)`);
for (const [hours, minutes, want] of [[5, 59, false], [6, 0, false], [6, 1, true]]) {
  const m = machine();
  m.advance(hours * HOUR + minutes * 60_000);
  const step = m.send("input");
  check(step.isReturn === want, `${hours} h ${String(minutes).padStart(2, "0")} m is ${want ? "a return" : "not a return"}`);
}
for (const event of ["visible", "focus", "input"]) {
  const m = machine();
  m.advance(7 * HOUR);
  check(m.send(event).isReturn, `the first ${event} after seven hours is a return`);
}
{
  const m = machine();
  const since = m.state().lastActive;
  m.advance(7 * HOUR);
  const visible = m.send("visible");
  const focus = m.send("focus");
  const input = m.send("input");
  check(visible.isReturn && !focus.isReturn && !input.isReturn,
    "visible, focus and input after one gap produce exactly one return");
  check(since === BASE && visible.state.lastActive === m.now(),
    "the return's since is the old lastActive, and the visible return itself advances activity to now");
}

{
  // No hide or visibility transition: the laptop slept with the window showing.
  const m = machine();
  m.advance(9 * HOUR);
  const step = m.send("input");
  check(step.isReturn && step.state.lastActive === m.now() && step.persist,
    "the first input after a sleep with the window visible is a return, and its long-overdue activity is persisted");
}
{
  const m = machine();
  m.advance(7 * HOUR);
  const first = m.send("visible");
  check(first.isReturn && first.state.shownDay === YESTERDAY, "a return alone does not mark the day: an unchanged digest can try again later");
  m.mark();
  m.advance(7 * HOUR);
  check(m.state().shownDay === TODAY && !m.send("focus").isReturn,
    "once the panel is marked shown, even another qualifying gap that day is not a return");
}
{
  const m = machine();
  m.advance(7 * HOUR);
  m.send("visible");
  m.advance(7 * HOUR);
  check(m.send("focus").isReturn, "without the panel being shown, another qualifying gap that day is still a return");
}
{
  const m = machine({ now: Date.parse("2026-11-27T18:00:00Z"), lastActive: Date.parse("2026-11-27T18:00:00Z") });
  m.advance(7 * HOUR);
  check(m.send("visible", "2026-11-28").isReturn, "a seven-hour return across local midnight is a return on the new day");
}
{
  const m = machine();
  check(R.INPUT_PERSIST_MS === 10 * 60_000, "input persistence has one named ten-minute threshold");
  m.advance(5 * 60_000);
  check(!m.send("input").persist, "an input five minutes after the persisted activity is not written");
  m.advance(5 * 60_000);
  check(!m.send("input").persist, "exactly ten minutes after the persisted activity is still not written");
  m.advance(60_000);
  const saved = m.send("input");
  check(saved.persist && saved.state.lastPersisted === m.now(),
    "an input more than ten minutes past the persisted activity is written and moves the in-memory persisted stamp");
  m.advance(60_000);
  check(!m.send("input").persist, "the next input is not another write");
}
{
  const m = machine({ lastActive: null, shownDay: null });
  check(!m.send("visible").isReturn && !m.send("focus").isReturn, "a first run with no stored lastActive is not a return");
  const input = m.send("input");
  check(!input.isReturn && input.state.lastActive === BASE, "the first input establishes activity without inventing a return");
}
{
  const m = machine();
  m.advance(30 * 60_000);
  const input = m.send("input");
  m.advance(30 * 60_000);
  const hidden = m.send("hidden");
  m.advance(10 * 60_000);
  const unload = m.send("unload");
  check(input.persist && hidden.persist && unload.persist, "a throttled input, hidden and unload ask for persistence");
  check(hidden.state.lastActive === BASE + HOUR && unload.state.lastActive === hidden.state.lastActive,
    "lastActive follows the later input/hide, while unload saves it without counting as fresh activity");
}

if (failed) {
  console.log(`\n${failed} failed`);
  process.exit(1);
}
console.log("\nreturning-check: all ok");
