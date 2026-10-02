// Deterministic check of the hourly release poll (`updatepoll.ts`) and of the
// rule that decides whether the update bubble shows.
//
// The poll used to be one `setTimeout(checkUpdate, 3000)` at launch, so an app
// left open for a week never heard of a release. It is now a schedule, and a
// schedule has exactly the bugs no suite can see: a check that fires while the
// window is hidden (AGENTS.md: a hidden tab never polls), two checks stacked
// because a slow `curl` was still in flight when the timer or a visibility
// change asked again, a retry storm while offline, or a timer that outlives the
// toggle that started it. The mock answers in a microtask and nobody waits an
// hour in the harness, so all of these pass every other gate.
//
// Same approach as termresize-check.mjs: it evaluates the REAL module source,
// and time is VIRTUAL — the poller takes its clock and timers from `env`, so a
// clock this file owns drives it exactly. Real timers would make an hour-long
// schedule untestable and a short one flaky.
//
//   node updatepoll-check.mjs [path/to/updatepoll.ts]   exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const SRC = process.argv[2] || fileURLToPath(new URL("../src/updatepoll.ts", import.meta.url));
const raw = fs.readFileSync(SRC, "utf8");
// A data: URL has no base to resolve `./x` against, and an import would also
// mean the schedule depends on something this check does not evaluate.
if (/^\s*import\s(?!type\b)/m.test(raw)) throw new Error(`${SRC}: must stay import-free (see header)`);
const js = (await transformWithEsbuild(raw, "updatepoll.ts", { loader: "ts", format: "esm" })).code;
const M = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const ok = (cond, msg) => { if (!cond) { failed++; console.error("FAIL:", msg); } else console.log("ok:", msg); };

const MIN = 60_000, HOUR = 60 * MIN;
ok(M.UPDATE_FIRST_MS === 3000, "first check stays ~3s after launch, off the startup path");
ok(M.UPDATE_EVERY_MS === HOUR, "steady cadence is hourly");

// ── virtual clock + controllable visibility + a scripted check ───────────────
function rig({ visible = true } = {}) {
  let now = 0, seq = 0, vis = visible;
  const timers = new Map();
  const visSubs = new Set();
  const calls = [];          // virtual times at which check() was CALLED
  let pending = [];          // resolvers of in-flight checks
  let answer = true;         // what the next check resolves to (or "throw" / "hang")
  const env = {
    now: () => now,
    setTimeout(fn, ms) { const id = ++seq; timers.set(id, { at: now + ms, fn }); return id; },
    clearTimeout(id) { timers.delete(id); },
    visible: () => vis,
    onVisible(fn) { visSubs.add(fn); return () => visSubs.delete(fn); },
  };
  const check = () => {
    calls.push(now);
    if (answer === "throw") return Promise.reject(new Error("offline"));
    return new Promise((r) => { if (answer === "hang") pending.push(r); else r(answer); });
  };
  const flush = () => new Promise((r) => globalThis.setTimeout(r, 0));
  return {
    env, check, calls, visSubs, timers,
    set answer(a) { answer = a; },
    async advance(ms) {
      const target = now + ms;
      for (;;) {
        let next = null;
        for (const [id, t] of timers) if (t.at <= target && (!next || t.at < next.t.at)) next = { id, t };
        if (!next) break;
        timers.delete(next.id);
        now = next.t.at;
        next.t.fn();
        await flush(); await flush();
      }
      now = target;
      await flush(); await flush();
    },
    async setVisible(v) { vis = v; for (const f of visSubs) f(); await flush(); await flush(); },
    async resolveAll(v) { const p = pending; pending = []; p.forEach((r) => r(v)); await flush(); await flush(); },
  };
}

// 1. Launch: nothing before 3s, one check at 3s, then hourly.
{
  const r = rig();
  const stop = M.startUpdatePoll(r.check, r.env);
  await r.advance(2999);
  ok(r.calls.length === 0, "no check before 3s");
  await r.advance(1);
  ok(r.calls.length === 1 && r.calls[0] === 3000, "first check at exactly 3s");
  await r.advance(HOUR - 1);
  ok(r.calls.length === 1, "no second check before the hour is up");
  await r.advance(1);
  ok(r.calls.length === 2 && r.calls[1] === 3000 + HOUR, "second check one hour after the first COMPLETED");
  await r.advance(5 * HOUR);
  ok(r.calls.length === 7, `steady hourly over 5 more hours (got ${r.calls.length - 2} more)`);
  stop();
}

// 2. Hidden: a due check is held, not run; regaining visibility runs it ONCE.
{
  const r = rig();
  const stop = M.startUpdatePoll(r.check, r.env);
  await r.advance(3000);                 // first check, visible
  await r.setVisible(false);
  await r.advance(3 * HOUR);
  ok(r.calls.length === 1, "no check at all while hidden, however long");
  await r.setVisible(true);
  ok(r.calls.length === 2 && r.calls[1] === 3000 + 3 * HOUR, "regaining visibility after the hour checks immediately");
  await r.setVisible(false); await r.setVisible(true);
  await r.setVisible(false); await r.setVisible(true);
  ok(r.calls.length === 2, "flapping visibility does not re-check inside the hour");
  await r.advance(HOUR);
  ok(r.calls.length === 3, "and the hourly cadence resumes from that check");
  stop();
}

// 3. Visible again BEFORE the hour: no early check, the original due time holds.
{
  const r = rig();
  const stop = M.startUpdatePoll(r.check, r.env);
  await r.advance(3000);
  await r.setVisible(false);
  await r.advance(20 * MIN);
  await r.setVisible(true);
  ok(r.calls.length === 1, "regaining visibility inside the hour does not check");
  await r.advance(40 * MIN);
  ok(r.calls.length === 2 && r.calls[1] === 3000 + HOUR, "the check still lands on the hour");
  stop();
}

// 4. Launched hidden (window opened behind another Space): the first check
//    waits for visibility rather than firing into a hidden window.
{
  const r = rig({ visible: false });
  const stop = M.startUpdatePoll(r.check, r.env);
  await r.advance(10 * MIN);
  ok(r.calls.length === 0, "a hidden launch does not check");
  await r.setVisible(true);
  ok(r.calls.length === 1, "first visibility runs the overdue launch check");
  stop();
}

// 5. Offline: back off 5, 10, 20, 40, then cap at the hour; success resets.
{
  const r = rig();
  r.answer = false;
  const stop = M.startUpdatePoll(r.check, r.env);
  await r.advance(3000);
  await r.advance(8 * HOUR);
  const gaps = r.calls.slice(1).map((t, i) => (t - r.calls[i]) / MIN);
  ok(JSON.stringify(gaps.slice(0, 6)) === JSON.stringify([5, 10, 20, 40, 60, 60]),
    `failure backoff 5,10,20,40,60,60 min (got ${gaps.slice(0, 6).join(",")})`);
  ok(gaps.every((g) => g <= 60), "never slower than hourly while failing");
  r.answer = true;
  const n = r.calls.length;
  await r.advance(HOUR);                 // next (capped) attempt succeeds
  await r.advance(5 * MIN);
  ok(r.calls.length === n + 1, "after a success the 5-minute retry does not come back");
  await r.advance(HOUR);
  ok(r.calls.length === n + 2, "success restores the hourly cadence");
  stop();
}

// 6. A rejected check counts as a failure, not a crash that ends the schedule.
{
  const r = rig();
  r.answer = "throw";
  const stop = M.startUpdatePoll(r.check, r.env);
  await r.advance(3000);
  await r.advance(5 * MIN);
  ok(r.calls.length === 2, "a throwing check is retried on the backoff");
  stop();
}

// 7. Never stack: while a check is in flight nothing starts another — not the
//    visibility handler, not a timer.
{
  const r = rig();
  r.answer = "hang";
  const stop = M.startUpdatePoll(r.check, r.env);
  await r.advance(3000);
  ok(r.calls.length === 1, "first check in flight");
  await r.setVisible(false); await r.setVisible(true);
  await r.advance(3 * HOUR);
  await r.setVisible(false); await r.setVisible(true);
  ok(r.calls.length === 1, "a hung check is never joined by a second one");
  r.answer = true;
  await r.resolveAll(true);
  await r.advance(HOUR);
  ok(r.calls.length === 2, "once it answers, the schedule continues from there");
  stop();
}

// 8. stop(): no timer survives it, nothing subscribes, and an in-flight check
//    that answers afterwards does not re-arm anything.
{
  const r = rig();
  r.answer = "hang";
  const stop = M.startUpdatePoll(r.check, r.env);
  await r.advance(3000);
  stop();
  ok(r.visSubs.size === 0, "stop() unsubscribes from visibility");
  await r.resolveAll(true);
  ok(r.timers.size === 0, "an answer landing after stop() arms no timer");
  await r.advance(10 * HOUR);
  await r.setVisible(true);
  ok(r.calls.length === 1, "no check after stop()");
}

// 9. Who sees the bubble. Per VERSION: a dismissed tag stays dismissed, a newer
//    one comes back; a CLI that is merely missing is not a release.
{
  const t = M.bubbleTag;
  ok(t({ latest: "v0.36.0", newer: true, dismissed: undefined }) === "v0.36.0", "newer release, never dismissed → shows");
  ok(t({ latest: "v0.36.0", newer: true, dismissed: "v0.36.0" }) === null, "dismissed tag stays dismissed");
  ok(t({ latest: "v0.37.0", newer: true, dismissed: "v0.36.0" }) === "v0.37.0", "a NEWER release re-shows");
  ok(t({ latest: "v0.36.0", newer: false, dismissed: undefined }) === null, "nothing newer → no bubble");
  ok(t({ latest: null, newer: false, dismissed: undefined }) === null, "offline → no bubble");
}

// 10. The wiring: App drives the poll through this module, gated on the
//     setting, and the old launch-only timer is gone.
{
  const app = fs.readFileSync(fileURLToPath(new URL("../src/App.tsx", import.meta.url)), "utf8");
  ok(/from "\.\/updatepoll"/.test(app), "App imports updatepoll");
  ok(!/setTimeout\(checkUpdate,\s*3000\)/.test(app), "the launch-only setTimeout(checkUpdate, 3000) is gone");
  const eff = app.match(/useEffect\(\(\) => \{\s*if \(!settings\.update_auto_check\) return;[\s\S]*?\}, \[([^\]]*)\]\);/);
  ok(!!eff && /startUpdatePoll\(/.test(eff[0]), "the poll starts inside the effect gated on update_auto_check");
  ok(!!eff && /settings\.update_auto_check/.test(eff[1]), "…and that effect re-runs (so stops) when the toggle changes");
}

if (failed) { console.error(`\n${failed} failure(s)`); process.exit(1); }
console.log("\nupdatepoll-check: all passed");
