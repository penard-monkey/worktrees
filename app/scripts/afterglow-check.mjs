// Guards the afterglow decay curve — the arithmetic behind the purple dot.
//
//   node app/scripts/afterglow-check.mjs        # exits non-zero on failure
//
// The tiers used to be three constants in App.tsx and three CSS rules, which is
// a shape a reviewer can check by reading. They are now a GEOMETRIC series
// derived from two user settings, which is not: `doneBounds` can return an
// array that is the wrong length, not ascending, off by one at the ends, or
// full of NaN for a hand-edited ui-state.json, and every one of those renders
// as a nav that simply looks a bit different. Nothing else can see it — the
// bats suite does not run the frontend, tsc types `number[]` and stops there,
// and the mock harness cannot express "is this curve the one that was meant".
//
// So this drives the REAL `src/afterglow.ts` (compiled through esbuild, the
// same technique as zoom-check.mjs and race-check.mjs — it tests the edit, not
// a paraphrase of it) and then checks the two places the module's output has to
// land: the stylesheet must no longer carry per-tier opacities, and App.tsx
// must no longer carry a boundary of its own. One rule, one home.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const eq = (got, want, what) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
};

const load = async (src, name) => {
  const js = (await transformWithEsbuild(src, name, { loader: "ts", format: "esm" })).code;
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
};

// afterglow.ts imports nothing — that is deliberate (App.tsx, SettingsSheet.tsx
// and this script all need it), so it loads under node with no stubbing at all.
const A = await load(read("../src/afterglow.ts"), "afterglow.ts");
const { DONE_FIRST_SECS, DONE_HORIZONS, DONE_STEPS_MIN, DONE_STEPS_MAX,
        doneBounds, doneTier, doneOpacity, fmtSecs, snapHorizon, clampSteps } = A;

if (DONE_FIRST_SECS !== 900)
  fail(`DONE_FIRST_SECS is ${DONE_FIRST_SECS}, not 900 — tier 1 is the "go look" window AND the project-folder rollup; both are statements about a quarter of an hour`);
if (DONE_STEPS_MIN < 2) fail(`DONE_STEPS_MIN is ${DONE_STEPS_MIN} — below 2 there is no ratio to space anything by`);
if (DONE_STEPS_MAX < DONE_STEPS_MIN) fail("DONE_STEPS_MAX < DONE_STEPS_MIN");

// ── the horizon table ──────────────────────────────────────────────────────
if (!DONE_HORIZONS.every((h, i) => i === 0 || h > DONE_HORIZONS[i - 1]))
  fail(`DONE_HORIZONS is not strictly ascending (${JSON.stringify(DONE_HORIZONS)}) — the slider walks it by index`);
if (DONE_HORIZONS[0] <= DONE_FIRST_SECS)
  fail(`DONE_HORIZONS starts at ${DONE_HORIZONS[0]}, at or below the pinned first boundary ${DONE_FIRST_SECS} — the curve would run backwards`);
// Every stop must be a fixed point of the snap, or the slider's own value round
// trips into a DIFFERENT stop and the thumb jumps under the user's finger.
for (const h of DONE_HORIZONS) {
  if (snapHorizon(h) !== h) fail(`snapHorizon(${h}) = ${snapHorizon(h)} — a table stop that does not survive its own sanitiser`);
}

// ── the curve, over every combination the UI can produce ───────────────────
const NOW = 1_000_000_000;
let combos = 0;
for (const h of DONE_HORIZONS) {
  for (let n = DONE_STEPS_MIN; n <= DONE_STEPS_MAX; n++) {
    combos++;
    const b = doneBounds(h, n);
    const tag = `doneBounds(${h}, ${n})`;
    if (b.length !== n) { fail(`${tag}: length ${b.length}, want ${n}`); continue; }
    if (!b.every((x) => Number.isFinite(x) && Number.isInteger(x))) { fail(`${tag}: not all whole finite seconds — ${JSON.stringify(b)}`); continue; }
    if (b[0] !== DONE_FIRST_SECS) fail(`${tag}: first bound ${b[0]}, want the pinned ${DONE_FIRST_SECS}`);
    if (b[n - 1] !== h) fail(`${tag}: last bound ${b[n - 1]}, want the horizon ${h}`);
    if (!b.every((x, i) => i === 0 || x > b[i - 1])) fail(`${tag}: not strictly increasing — ${JSON.stringify(b)}`);

    // Geometric, within the rounding to whole seconds: each bound must be the
    // rounded exact term, and each neighbour ratio the same constant.
    const r = Math.pow(h / DONE_FIRST_SECS, 1 / (n - 1));
    for (let i = 0; i < n; i++) {
      const exact = DONE_FIRST_SECS * Math.pow(r, i);
      if (Math.abs(b[i] - exact) > 0.5 + 1e-6)
        fail(`${tag}: b[${i}] = ${b[i]}, exact geometric term is ${exact.toFixed(3)} — spacing is not geometric`);
    }
    for (let i = 0; i + 1 < n; i++) {
      const got = b[i + 1] / b[i];
      // Worst case from rounding both ends by half a second.
      const tol = r * (0.5 / b[i] + 0.5 / b[i + 1]) + 1e-9;
      if (Math.abs(got - r) > tol)
        fail(`${tag}: ratio b[${i + 1}]/b[${i}] = ${got.toFixed(6)}, want ${r.toFixed(6)} ±${tol.toExponential(2)}`);
    }

    // Tiers: just under a bound is that bound's tier, exactly at it is the next.
    for (let i = 0; i < n; i++) {
      const under = doneTier(NOW - (b[i] - 1), NOW, b);
      if (under !== i + 1) fail(`${tag}: age ${b[i] - 1}s (1s under b[${i}]) is tier ${under}, want ${i + 1}`);
      const at = doneTier(NOW - b[i], NOW, b);
      const want = i + 1 < n ? i + 2 : 0;
      if (at !== want) fail(`${tag}: age ${b[i]}s (exactly b[${i}]) is tier ${at}, want ${want}`);
    }
    if (doneTier(NOW - h, NOW, b) !== 0) fail(`${tag}: age === the horizon still lights a dot`);
    if (doneTier(NOW - h - 1, NOW, b) !== 0) fail(`${tag}: age past the horizon still lights a dot`);
    if (doneTier(NOW + 3600, NOW, b) !== 1) fail(`${tag}: a stamp from the future (clock skew) is not tier 1`);
    if (doneTier(0, NOW, b) !== 0) fail(`${tag}: a place with no last_worked_epoch lights a dot`);

    // Opacity: monotone down across the tiers, never outside 0..1.
    const ops = Array.from({ length: n }, (_, i) => doneOpacity(i + 1, n));
    if (ops[0] !== 1) fail(`doneOpacity(1, ${n}) = ${ops[0]} — tier 1 must be full brightness`);
    if (!ops.every((o, i) => o > 0 && o <= 1 && (i === 0 || o < ops[i - 1])))
      fail(`doneOpacity over ${n} steps is not monotone decreasing inside (0,1] — ${JSON.stringify(ops)}`);
  }
}

// The default curve is the one the old three constants stood for, and the one
// the mock fixtures (4min / 45min / 5h) are written against.
eq(doneBounds(12 * 3600, 3), [900, 6235, 43200], "the default 12h / 3-step curve");
eq([doneOpacity(1, 3), doneOpacity(2, 3), doneOpacity(3, 3)], [1, 0.65, 0.3],
   "3 steps must reproduce the opacities the stylesheet used to hardcode");

// ── garbage in ─────────────────────────────────────────────────────────────
// ui-state.json is a plain file the user can edit; none of this may throw, and
// none of it may produce an array the renderer would turn into `opacity: NaN`.
for (const [h, n] of [[NaN, NaN], [undefined, undefined], [null, null], [-1, 0], [0, 1],
                      [1e12, 999], [Infinity, -Infinity], ["12h", "three"], [12 * 3600, 3.6]]) {
  let b;
  try { b = doneBounds(h, n); } catch (e) { fail(`doneBounds(${String(h)}, ${String(n)}) threw: ${e}`); continue; }
  const tag = `doneBounds(${String(h)}, ${String(n)})`;
  if (!Array.isArray(b) || b.length < DONE_STEPS_MIN || b.length > DONE_STEPS_MAX)
    fail(`${tag}: length ${b && b.length} outside ${DONE_STEPS_MIN}..${DONE_STEPS_MAX}`);
  else if (!b.every((x) => Number.isFinite(x) && x > 0) || !b.every((x, i) => i === 0 || x > b[i - 1]))
    fail(`${tag}: ${JSON.stringify(b)} is not a usable ascending curve`);
  else if (b[0] !== DONE_FIRST_SECS || !DONE_HORIZONS.includes(b[b.length - 1]))
    fail(`${tag}: ${JSON.stringify(b)} does not start at ${DONE_FIRST_SECS} and end on a table stop`);
  const o = doneOpacity(99, n);
  if (!Number.isFinite(o) || o <= 0 || o > 1) fail(`doneOpacity(99, ${String(n)}) = ${o} — an out-of-range tier must still be a legal opacity`);
}
if (!Number.isFinite(clampSteps(NaN)) || !Number.isFinite(snapHorizon(NaN)))
  fail("clampSteps/snapHorizon let a NaN through — loadSettings would persist it");

// ── the label ──────────────────────────────────────────────────────────────
// The hint prints these; a carry bug reads as "1h 60m" and looks like a typo.
for (const [s, want] of [[900, "15m"], [6235, "1h 44m"], [43200, "12h"], [3600, "1h"],
                         [2 * 86400, "2d"], [7 * 86400, "7d"], [86399, "1d"], [3599, "1h"]]) {
  const got = fmtSecs(s);
  if (got !== want) fail(`fmtSecs(${s}) = "${got}", want "${want}"`);
}
for (const h of DONE_HORIZONS) {
  for (let n = DONE_STEPS_MIN; n <= DONE_STEPS_MAX; n++) {
    for (const b of doneBounds(h, n)) {
      const l = fmtSecs(b);
      if (/\b60m\b/.test(l) && !/^60m$/.test(l)) fail(`fmtSecs(${b}) = "${l}" — the minute carry is broken`);
      if (/\b24h\b/.test(l) && !/^24h$/.test(l)) fail(`fmtSecs(${b}) = "${l}" — the hour carry is broken`);
    }
  }
}

// ── the mirror must not grow back ──────────────────────────────────────────
const css = read("../src/App.css");
if (/\.done\.t2|\.done\.t3/.test(css))
  fail("App.css: a per-tier `.done.t2`/`.done.t3` opacity is back — the stylesheet cannot know the step count, so those rules are either dead or fighting the inline style");
if (!/\.status-dot\.done\.t1\s*\{[^}]*box-shadow/.test(css))
  fail("App.css: `.status-dot.done.t1`'s ring is gone — tier 1 loses the one channel that is not brightness");

const app = read("../src/App.tsx");
if (/DONE_T[123]_SECS/.test(app))
  fail("App.tsx: a `DONE_Tn_SECS` constant is back — a boundary living outside afterglow.ts is a second answer to the same question");
// Presence, which is all a static pass can know: the fade is an inline style
// now, so a render site that forgets it shows every tier at full brightness and
// nothing anywhere fails.
const sites = (app.match(/className=\{"status-dot" \+ dotClass\(p\)\}/g) || []).length;
const styled = (app.match(/className=\{"status-dot" \+ dotClass\(p\)\} style=\{dotStyle\(p\)\}/g) || []).length;
if (sites !== styled)
  fail(`App.tsx: ${sites} afterglow dot render site(s) but ${styled} carry style={dotStyle(p)} — the nav row and the Resume list must not drift`);
if (!/places\.some\(\(p\) => doneOf\(p\) === 1\)/.test(app))
  fail("App.tsx: the project rollup no longer keys on tier 1 — the folder would light for the whole horizon");

const settings = read("../src/settings.ts");
for (const k of ["done_horizon_secs", "done_steps"]) {
  if (!new RegExp(`${k}:`).test(settings)) fail(`settings.ts: \`${k}\` is missing from the Settings type/defaults`);
}
if (!/s\.done_horizon_secs = snapHorizon\(/.test(settings) || !/s\.done_steps = clampSteps\(/.test(settings))
  fail("settings.ts: loadSettings no longer sanitises the two afterglow keys — a hand-edited ui-state.json reaches the render unchecked");

if (bad) { console.error(`\nafterglow-check: ${bad} failure(s)`); process.exit(1); }
console.log(`afterglow-check: ok — ${combos} horizon×step curves verified, default ${JSON.stringify(doneBounds(12 * 3600, 3))} (${doneBounds(12 * 3600, 3).map(fmtSecs).join(" · ")}), mirrors clean`);
