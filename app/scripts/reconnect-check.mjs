// The engine's reconnect policy, on a virtual clock.
//
//   node app/scripts/reconnect-check.mjs        # from the repo root; exits non-zero on failure
//
// `app/src/reconnect.ts` decides when the page tries to reach its engine
// again and whether a try may START a serve; the shim only obeys `start:
// false`. Nothing else exercises it — the mock has no engine, and no Rust test
// sees a browser — so this check loads the REAL module (as a `data:` module,
// which is why it may have no runtime import) and drives it with a scripted
// connector on a virtual clock. Real timers would make every assertion here a
// bet on the scheduler (the termresize-check lesson); a virtual clock makes
// "the next try is 2000 ms later" a fact.
//
// Covered: the backoff sequence, its cap, the jitter's bounds (and that the
// machine applies it), the crash-loop limit and what a stop leaves behind,
// owned vs unowned, a manual try cancelling the timer, a hidden window
// deferring a due retry, and exactly one try in flight.
//
// Override, so a mutation can be tested without touching the tree:
//   RECONNECT_SRC=<path to a reconnect.ts>
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = process.env.RECONNECT_SRC ?? here("../src/reconnect.ts");
const src = fs.readFileSync(SRC, "utf8");

let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (cond, m) => (cond ? ok(m) : fail(m));
const tick = () => new Promise((r) => setTimeout(r, 0));
const ticks = async (n = 6) => { for (let i = 0; i < n; i++) await tick(); };
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

check(!/^\s*import\s(?!type\b)/m.test(src), "reconnect.ts has no runtime import (a data: module could not resolve one — and this check loads it as one)");
const js = (await transformWithEsbuild(src, path.basename(SRC), { loader: "ts", format: "esm" })).code;
const R = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

/** A clock that fires timers in due order, each with `now()` at its due time. */
function clock() {
  let now = 0, next = 1;
  const timers = new Map();
  return {
    now: () => now,
    setTimeout(fn, ms) { const id = next++; timers.set(id, { at: now + ms, ms, fn }); return id; },
    clearTimeout(id) { timers.delete(id); },
    advance(ms) {
      const end = now + ms;
      for (;;) {
        let first = null;
        for (const [id, t] of timers) if (t.at <= end && (!first || t.at < first[1].at)) first = [id, t];
        if (!first) break;
        timers.delete(first[0]);
        now = first[1].at;
        first[1].fn();
      }
      now = end;
    },
    pending: () => timers.size,
    /** The delay the one pending timer was set with. */
    armed: () => (timers.size === 1 ? [...timers.values()][0].ms : null),
  };
}

/** The machine under a virtual clock, a scripted connector and a switchable
 *  visibility. Each try waits until the test answers it. */
function rig({ random = () => 0, visible = true } = {}) {
  const clk = clock();
  const env = { now: clk.now, setTimeout: clk.setTimeout, clearTimeout: clk.clearTimeout, random: () => random(), visible: () => rig.visible };
  rig.visible = visible;
  const tries = [];
  const views = [];
  const m = R.retrier(env, (t, phase) => new Promise((resolve) => tries.push({ ...t, at: clk.now(), phase, resolve })), (v) => views.push(v));
  const r = {
    clk, m, tries, views,
    get visible() { return rig.visible; },
    set visible(v) { rig.visible = v; },
    view: () => views[views.length - 1],
    last: () => tries[tries.length - 1],
    /** Answer the latest try, then let the machine see it. */
    async answer(outcome) { r.last().resolve(outcome); await ticks(); },
    async up(owned = true, started = false) { await r.answer({ ok: true, owned, started }); },
    async failed(reason = "refused", { started = false, declined = false } = {}) { await r.answer({ ok: false, reason, started, declined }); },
    /** Boot into "up". */
    async booted(owned = true, started = true) { void m.boot(); await ticks(); await r.up(owned, started); },
  };
  return r;
}

const SEQ = [500, 1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000];

// 1. The backoff sequence, and the cap.
{
  const h = rig();
  await h.booted();
  h.m.lost("the connection to worktrees serve closed");
  const delays = [];
  for (let i = 0; i < SEQ.length; i++) {
    const v = h.view();
    delays.push(v.kind === "waiting" ? v.at - h.clk.now() : `<${v.kind}>`);
    const before = h.tries.length;
    h.clk.advance(SEQ[i] - 1);
    const early = h.tries.length !== before;
    h.clk.advance(1);
    if (early || h.tries.length !== before + 1) { fail(`retry ${i} did not land exactly ${SEQ[i]} ms after the last (early: ${early}, tries ${h.tries.length - before})`); break; }
    await ticks();
    await h.failed("refused");
  }
  check(same(delays, SEQ), `with no jitter the retries wait 0.5, 1, 2, 4, 8, 16, then 30 s for good (${JSON.stringify(delays)})`);
  check(h.tries.slice(1).every((t) => t.reason !== null && t.start && !t.manual), "every automatic retry carries why it is retrying, is not manual, and (the app's serve, budget unspent) may start");
  check(same(h.tries.map((t) => t.attempt), [1, ...SEQ.map((_, i) => i + 1)]), `the shim is told each try's number since the engine was last up, from 1 (${h.tries.map((t) => t.attempt).join(",")})`);
  const big = [6, 7, 20, 60, 1100].map((n) => R.nominalDelay(n));
  check(big.every((d) => d === R.BACKOFF.capMs), `the cap holds from the 7th retry on, however many there are — no Infinity, no NaN (${big.join(", ")})`);
  check(R.BACKOFF.baseMs === 500 && R.BACKOFF.capMs === 30000, `the numbers are the plan's: base ${R.BACKOFF.baseMs} ms, cap ${R.BACKOFF.capMs} ms`);
  h.clk.advance(30_000); await ticks();
  await h.up(true);
  check(h.view().kind === "up" && h.clk.pending() === 0, "a success clears the timer and reports up");
  h.m.lost("closed again");
  check(h.view().kind === "waiting" && h.view().at - h.clk.now() === 500, "…and the next loss starts the sequence over at 0.5 s");
}

// 2. Jitter: (d/2, d], and the machine applies it.
{
  let bad = 0, lo = Infinity, hi = -Infinity;
  for (let n = 0; n < 9; n++) {
    const d = R.nominalDelay(n);
    for (let i = 0; i < 2000; i++) {
      const w = R.jittered(n, Math.random());
      if (!(w > d / 2 && w <= d)) bad++;
      lo = Math.min(lo, w / d); hi = Math.max(hi, w / d);
    }
  }
  check(bad === 0, `18000 draws over nine steps all land in (d/2, d] — never later than nominal, so the countdown is an upper bound (${bad} outside; ratio ${lo.toFixed(3)}..${hi.toFixed(3)})`);
  check(R.jittered(3, 0) === 4000 && R.jittered(3, 1 - 1e-12) > 2000 && R.jittered(3, 0.5) === 3000, "the bounds are exact: r=0 is nominal, r→1 approaches half, r=0.5 is three quarters");
  const h = rig({ random: () => 0.75 });
  await h.booted();
  h.m.lost("closed");
  const armed = h.clk.armed();
  check(armed === R.jittered(0, 0.75) && armed === 312.5, `the machine draws its jitter from env.random (first wait ${armed} ms for r=0.75)`);
  check(h.view().kind === "waiting" && h.view().at === h.clk.now() + armed, "…and the banner's `at` is the jittered time, not the nominal one");
}

// 3. The crash-loop limit.
{
  const h = rig();
  await h.booted(true, true); // the launch started serve: one start
  h.m.lost("the connection to worktrees serve closed");
  const exits = "worktrees serve exited before it was ready (exit status: 1)";
  h.clk.advance(500); await ticks();
  const t1 = h.last();
  await h.failed(exits, { started: true });
  h.clk.advance(1000); await ticks();
  const t2 = h.last();
  await h.failed(exits, { started: true });
  h.clk.advance(2000); await ticks();
  const t3 = h.last();
  check(t1.start && t2.start, "the 2nd and 3rd starts in a minute are allowed");
  check(!t3.start, `the 4th try inside 60 s may NOT start (start: ${t3.start}) — it may still reconnect, should a serve be there`);
  await h.failed("nothing to reconnect to: serve.json names pid 7, which is not running", { declined: true });
  const v = h.view();
  check(v.kind === "stopped" && /3 times in the last 60 s/.test(v.reason) && v.reason.includes(exits), `a declined try past the limit STOPS the retries and says why, naming the last failed start (${v.reason})`);
  check(h.clk.pending() === 0, "…with no timer left behind");
  const n = h.tries.length;
  h.clk.advance(10 * 60_000); await ticks();
  h.visible = false; h.m.visibility(); h.visible = true; h.m.visibility(); await ticks();
  check(h.tries.length === n, `nothing tries again on its own after a stop — not time, not visibility (${h.tries.length - n} tries)`);
  void h.m.now(); await ticks();
  const press = h.last();
  check(h.tries.length === n + 1 && press.manual && press.start, "a press of Reconnect now tries at once and may start one");
  await h.failed(exits, { started: true });
  check(h.view().kind === "waiting", "…a press whose start fails goes back to retrying");
  h.clk.advance(30_000); await ticks();
  const after = h.last();
  check(h.tries.length === n + 2 && !after.start, `…but the next automatic try may not start, though the window has long aged out: a press never re-arms automatic starts (start: ${after.start})`);
  await h.failed("nothing to reconnect to", { declined: true });
  check(h.view().kind === "stopped" && /on request did not stay up/.test(h.view().reason) && h.clk.pending() === 0, `…so one press is one start, and it stops again (${h.view().reason})`);
  void h.m.now(); await ticks();
  await h.up(true, true);
  h.m.lost("closed");
  h.clk.advance(500); await ticks();
  check(h.last().start, "once a press brings the engine up, a later loss may start one again");
  check(R.CRASH_LOOP.starts === 3 && R.CRASH_LOOP.windowMs === 60_000, `the limit is the plan's: ${R.CRASH_LOOP.starts} starts in ${R.CRASH_LOOP.windowMs / 1000} s`);
}
{
  // The window slides: three starts spread over more than a minute are not a loop.
  const h = rig();
  await h.booted(true, true); // t=0
  for (const gap of [40_000, 40_000]) {
    h.clk.advance(gap);
    h.m.lost("closed");
    h.clk.advance(500); await ticks();
    await h.up(true, true);
  }
  h.m.lost("closed");
  h.clk.advance(500); await ticks();
  check(h.last().start, "starts at 0 s, 40 s and 80 s leave the 4th try at 80.5 s free to start — only two are inside its minute");
}
{
  // At launch: a serve that can never come up stops after three starts.
  const h = rig();
  void h.m.boot(); await ticks();
  check(h.last().start && h.last().reason === null && !h.last().manual, "the launch's try may start one, and has no reason (nothing was lost)");
  for (const ms of [0, 500, 1000]) { h.clk.advance(ms); await ticks(); await h.failed("could not start worktrees", { started: true }); }
  h.clk.advance(2000); await ticks();
  check(!h.last().start, "a launch whose serve never comes up is held to the same three starts");
  await h.failed("nothing to reconnect to", { declined: true });
  check(h.view().kind === "stopped" && h.clk.pending() === 0, "…and stops");
}

// 4. Owned vs unowned.
{
  const h = rig();
  await h.booted(false, false); // someone else's serve
  h.m.lost("the connection to worktrees serve closed");
  const starts = [];
  for (const ms of [500, 1000, 2000, 4000]) {
    h.clk.advance(ms); await ticks();
    starts.push(h.last().start);
    await h.failed("nothing to reconnect to: no serve.json", { declined: true });
  }
  check(starts.every((s) => s === false), `a serve someone else started is never restarted: every automatic try is start:false (${starts.join(",")})`);
  check(h.view().kind === "waiting" && h.view().notOurs && h.clk.pending() === 1, "…and the retries go on, reconnect-only, saying the serve was not the app's");
  h.clk.advance(8000); await ticks();
  await h.up(false, false);
  check(h.view().kind === "up", "…until it is back");
  h.m.lost("closed");
  h.clk.advance(500); await ticks();
  check(!h.last().start, "it is still someone else's after coming back");
  await h.failed("nothing to reconnect to", { declined: true });
  void h.m.now(); await ticks();
  check(h.tries.length === 8 && h.last().manual && h.last().start, "a press may start the app's own in its place");
  await h.up(true, true);
  h.m.lost("closed");
  h.clk.advance(500); await ticks();
  check(h.last().start, "…and from then the serve is the app's: a loss may start one");
}
{
  const h = rig();
  await h.booted(true, false); // adopted: owned, not started
  h.m.lost("closed");
  h.clk.advance(500); await ticks();
  check(h.last().start, "an adopted serve is the app's: a loss may start one");
}

// 5. A press of Reconnect now cancels the pending timer.
{
  const h = rig();
  await h.booted();
  h.m.lost("closed");
  h.clk.advance(400);
  check(h.clk.pending() === 1 && h.tries.length === 1, "a retry is pending");
  void h.m.now(); await ticks();
  check(h.tries.length === 2 && h.last().manual && h.last().start && h.last().at === 400, "Reconnect now tries at once, not when the timer would have");
  check(h.clk.pending() === 0, "…and the pending timer is gone");
  await h.failed("refused");
  h.clk.advance(100); await ticks(); // where the cancelled 500 ms timer would have fired
  check(h.tries.length === 2, `the cancelled timer never fires a second try (${h.tries.length - 2} extra)`);
  check(h.view().kind === "waiting" && h.clk.pending() === 1, "a failed press goes back to the backoff");
  const n = h.tries.length;
  void h.m.now(); await ticks();
  await h.up();
  check(h.tries.length === n + 1 && h.clk.pending() === 0, "a press that succeeds leaves nothing pending");
  await h.m.now(); await ticks();
  check(h.tries.length === n + 1, "a press while up does nothing");
}

// 6. A hidden window defers a due retry; being seen runs it once.
{
  const h = rig();
  await h.booted();
  h.visible = false;
  h.m.lost("closed");
  h.clk.advance(500); await ticks();
  check(h.tries.length === 1 && h.view().kind === "hidden" && h.clk.pending() === 0, "a retry that falls due while hidden does not try, and says it waits to be seen");
  h.clk.advance(60 * 60_000); await ticks();
  check(h.tries.length === 1, "an hour hidden: still nothing tried, nothing polled");
  h.visible = true;
  h.m.visibility(); await ticks();
  check(h.tries.length === 2 && !h.last().manual, "becoming visible runs the due retry at once");
  h.m.visibility(); h.m.visibility(); await ticks();
  check(h.tries.length === 2, "…once: more visibility events while it is in flight do nothing");
  await h.failed("refused");
  h.m.visibility(); await ticks();
  check(h.tries.length === 2 && h.view().kind === "waiting", "a visibility event with a retry merely pending does not jump the queue");
  h.visible = false;
  h.clk.advance(1000); await ticks();
  h.visible = true;
  h.m.visibility(); await ticks();
  check(h.tries.length === 3, "hidden → due → seen works again on the next step");
  await h.up();
  h.m.visibility(); await ticks();
  check(h.tries.length === 3, "being seen while up does nothing");
}
{
  const h = rig({ visible: false });
  void h.m.boot(); await ticks();
  check(h.tries.length === 1, "the launch's own try is not deferred by a hidden window");
}

// 7. Exactly one try in flight.
{
  const h = rig();
  await h.booted();
  h.m.lost("closed");
  h.clk.advance(500); await ticks();
  check(h.tries.length === 2 && h.clk.pending() === 0, "a try is in flight, and no timer runs beside it");
  const p1 = h.m.now(), p2 = h.m.now();
  h.m.visibility();
  h.m.lost("a second loss");
  await ticks();
  check(h.tries.length === 2 && p1 === p2, `presses, visibility and a loss during a try start no second one (${h.tries.length - 2} extra)`);
  h.last().phase("waiting-old");
  check(h.view().kind === "trying" && h.view().phase === "waiting-old" && !h.view().manual, "the shim's phase reaches the banner while the try is in flight");
  await h.up();
  check(h.view().kind === "waiting", "a loss reported during the try is applied once the try has said up — the socket it made live is gone");
  const stale = h.tries[1].phase;
  stale("starting");
  check(h.view().kind === "waiting", "a phase from a finished try is ignored");
  h.clk.advance(500); await ticks();
  check(h.tries.length === 3, "…and the retry it scheduled runs");
  await h.up();
  h.m.lost("closed");
  void h.m.now(); await ticks();
  void h.m.now(); await ticks();
  check(h.tries.length === 4, "two presses in a row while one is in flight are one try");
}
{
  // A connector that throws is a failed try, not a stuck machine.
  const clk = clock();
  const views = [];
  let n = 0;
  const m = R.retrier({ now: clk.now, setTimeout: clk.setTimeout, clearTimeout: clk.clearTimeout, random: () => 0, visible: () => true },
    () => { n++; return Promise.reject(new Error("dial blew up")); }, (v) => views.push(v));
  await m.boot(); await ticks();
  check(views[views.length - 1].kind === "waiting" && /dial blew up/.test(views[views.length - 1].reason), "a connector that throws reads as a failed try, and the retries go on");
  clk.advance(500); await ticks();
  check(n === 2, "…on the backoff");
}

console.log(failed ? `\nreconnect-check: ${failed} failure(s)` : "\nreconnect-check: all good");
process.exit(failed ? 1 : 0);
