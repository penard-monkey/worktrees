// Exercise the real normalization and polling hook with a virtual scheduler.
import fs from "node:fs";
import assert from "node:assert/strict";
import React, { Fragment } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { transformWithEsbuild } from "vite";
const src = fs.readFileSync(new URL("../src/planUsage.ts", import.meta.url), "utf8");
const js = (await transformWithEsbuild(src, "planUsage.ts", { loader: "ts" })).code;
const model = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
// The meter's clock tick, sliced from App.tsx rather than restated: the slack
// above has to outlast it, and a copy here would let the two drift apart.
const app = fs.readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const ticks = [...app.matchAll(/^const USAGE_TICK_MS = ([\d_]+);/gm)];
assert.equal(ticks.length, 1, "expected exactly one USAGE_TICK_MS in App.tsx");
const USAGE_TICK_MS = Number(ticks[0][1].replaceAll("_", ""));
const { adaptCodex, adaptClaude, adaptKimi, viewUsage, summaryLimit, windowLabel, checking, expired, CODEX_STALE_SECS, KIMI_STALE_SECS, compactUsage } = model;
let checks = 0;
function test(name, fn) { fn(); checks++; console.log(`ok ${name}`); }
const limit = (role, minutes, percent, reset, bucket = "codex") => ({ id: `${bucket}:${role}`, bucket_id: bucket,
  bucket_label: bucket, window_role: role, window_minutes: minutes, percent, severity: "normal", resets_at: reset });
const wire = { state: "ready", source: "app_server", fetched_at: 100, reason: null,
  limits: [limit("primary", 10080, 24, 110), limit("secondary", null, 21, 3000)] };
// The reader lives in CORE, not under `app/src-tauri`: the CLI and MCP read
// usage too, and nothing outside the app can reach `app/src-tauri`. This path
// moving is what makes the check go red on a move rather than silently stop
// mirroring anything — `readFileSync` throws, it does not return "".
//
// Slice off the tests MODULE, not every `#[cfg(test)]`. The file can carry
// `#[cfg(test)] impl` fixture constructors ABOVE the production code, and
// cutting at the first marker truncates this mirror to the file's first lines
// — which happened to fail loudly, but a pattern that appeared in the head
// would have passed just as silently. Same family as docsviewer-check's
// first-match trap. Asserting the marker keeps a rename from quietly handing
// the whole file, fixtures included, to the patterns below.
const SRC = fs.readFileSync(new URL("../../crates/worktrees-core/src/codex_usage.rs", import.meta.url), "utf8");
const TESTS_AT = SRC.search(/^#\[cfg\(test\)\]\s*\nmod tests\b/m);
assert.notEqual(TESTS_AT, -1, "expected a `#[cfg(test)] mod tests` marker in codex_usage.rs");
const rust = SRC.slice(0, TESTS_AT);
test("Codex age ceiling mirrors the backend constant", () => {
  const definitions = [...rust.matchAll(/^const STALE: i64 = ([\d_]+);/gm)];
  assert.equal(definitions.length, 1, "expected exactly one backend STALE definition");
  assert.equal(CODEX_STALE_SECS, Number(definitions[0][1].replaceAll("_", "")));
});
test("Codex reset boundary mirrors the backend expiry rule", () => {
  const rules = [...rust.matchAll(/\.any\(\|l\| l\.resets_at\.is_some_and\(\|t\| t (<=|<) now\)\)/g)];
  assert.equal(rules.length, 1, "backend reset rule changed; review the frontend mirror");
  const info = adaptCodex(wire);
  for (const reset of [null, 99, 100, 101]) {
    const backend = reset !== null && (rules[0][1] === "<=" ? reset <= 100 : reset < 100);
    assert.equal(expired(info, { ...info.limits[0], resets_at: reset }, 100), backend);
  }
});
// kimi's reader (core `kimi_usage`) ages its answer like Codex's; the frontend
// mirror must agree with it, sliced the same way.
const KSRC = fs.readFileSync(new URL("../../crates/worktrees-core/src/kimi_usage.rs", import.meta.url), "utf8");
const KTESTS_AT = KSRC.search(/^#\[cfg\(test\)\]\s*\nmod tests\b/m);
assert.notEqual(KTESTS_AT, -1, "expected a `#[cfg(test)] mod tests` marker in kimi_usage.rs");
const krust = KSRC.slice(0, KTESTS_AT);
const kimiWire = { provider: "kimi", state: "ready", source: "kimi_web", fetched_at: 100, reason: null, limits: [
  { id: "kimi:limit5h", bucket_id: "kimi", bucket_label: "Kimi", window_role: "5h", window_minutes: 300, percent: 41, severity: "normal", resets_at: 110 },
  { id: "kimi:monthTotal", bucket_id: "kimi", bucket_label: "Kimi", window_role: "month", window_minutes: null, percent: 12, severity: "normal", resets_at: 5000 }] };
test("kimi age ceiling mirrors the backend constant", () => {
  const defs = [...krust.matchAll(/^const STALE: i64 = ([\d_]+);/gm)];
  assert.equal(defs.length, 1, "expected exactly one STALE in kimi_usage.rs");
  assert.equal(KIMI_STALE_SECS, Number(defs[0][1].replaceAll("_", "")));
});
test("the cadence the app and CHANGELOG promise (\"two minutes\") is kimi_usage's TTL and failure floor", () => {
  const ttl = [...krust.matchAll(/^const TTL: u64 = (\d+);/gm)];
  const floor = [...krust.matchAll(/^const FAILURE_FLOOR: u64 = (\d+);/gm)];
  assert.equal(ttl.length, 1, "expected exactly one TTL in kimi_usage.rs");
  assert.equal(floor.length, 1, "expected exactly one FAILURE_FLOOR in kimi_usage.rs");
  assert.equal(Number(ttl[0][1]), 120, "the Settings hint and CHANGELOG say two minutes");
  assert.equal(Number(floor[0][1]), 120, "…failures included");
  const sheet = fs.readFileSync(new URL("../src/SettingsSheet.tsx", import.meta.url), "utf8");
  assert.match(sheet, /for one request, at most every two minutes/);
});
test("kimi rows are labelled by their window and summarised from kimi's own bucket", () => {
  const info = adaptKimi(kimiWire);
  assert.deepEqual(info.limits.map(l => l.label), ["5h", "month"]);
  assert.equal(info.provider, "kimi");
  assert.equal(summaryLimit(info, 101).id, "kimi:limit5h");
  // A reset in the past stales only that row, as for Codex.
  const later = viewUsage(info, 111);
  assert.equal(later.state, "stale");
  assert.equal(summaryLimit(later, 111).id, "kimi:monthTotal");
  assert.equal(viewUsage(info, 101 + KIMI_STALE_SECS).limits.length, 0, "never a number past its age");
});
test("missing kimi has no compact slot either", () => {
  const missing = { ...checking("kimi"), state: "missing_cli" };
  assert.deepEqual(compactUsage([checking("claude"), missing]).map(i => i.provider), ["claude"]);
  assert.equal(compactUsage([{ ...missing, state: "signed_out" }]).length, 1);
});
test("missing Codex has no compact slot, while other states keep theirs", () => {
  const claude = checking("claude");
  const missing = { ...checking("codex"), state: "missing_cli" };
  assert.deepEqual(compactUsage([claude, missing]), [claude]);
  assert.deepEqual(compactUsage([missing]), []);
  for (const state of ["signed_out", "unsupported_auth", "unavailable"]) {
    assert.equal(compactUsage([{ ...missing, state }]).length, 1);
  }
});
test("duration labels come from the window, not its role", () => {
  assert.equal(windowLabel(10080, "primary"), "7d"); assert.equal(windowLabel(300, "secondary"), "5h");
  assert.equal(windowLabel(90, "primary"), "90m"); assert.equal(windowLabel(null, "primary"), "Primary");
});
test("reset invalidates only its own row between polls", () => {
  const info = viewUsage(adaptCodex(wire), 110);
  assert.equal(info.state, "stale"); assert.equal(expired(info, info.limits[0], 110), true);
  assert.equal(summaryLimit(info, 110).id, "codex:secondary");
});
test("age ceiling is inclusive and a bogus future timestamp is unavailable", () => {
  assert.equal(viewUsage(adaptCodex(wire), 100 + CODEX_STALE_SECS).limits.length, 2);
  assert.equal(viewUsage(adaptCodex(wire), 101 + CODEX_STALE_SECS).limits.length, 0);
  assert.equal(viewUsage(adaptCodex(wire), 99 - model.FUTURE_SLACK_SECS).state, "unavailable");
});
// The reader stamps `fetched_at` when it FINISHES, and `nowSec` moves only on
// its 15s tick, so a fresh reply is routinely a few seconds "in the future".
// That read as "unavailable" until the next tick (2026-10-22).
test("a reading fetched just after the local clock last ticked is fresh, not unavailable", () => {
  for (const [adapt, w] of [[adaptCodex, wire], [adaptKimi, kimiWire]]) {
    const info = adapt({ ...w, fetched_at: 105, limits: w.limits.map(l => ({ ...l, resets_at: 5000 })) });
    const seen = viewUsage(info, 100);
    assert.equal(seen.state, "ready", `${info.provider} fetched 5s after now`);
    assert.equal(seen.limits.length, 2);
    assert.equal(viewUsage(info, 105 - model.FUTURE_SLACK_SECS).state, "ready", "the slack is inclusive");
  }
  assert.ok(model.FUTURE_SLACK_SECS * 1000 > USAGE_TICK_MS, "the slack must cover a whole USAGE_TICK_MS");
});
test("reserve is never summarized as the main allowance", () => {
  const info = adaptCodex({ ...wire, limits: [limit("primary", 300, 99, 3000, "reserve")] });
  assert.equal(summaryLimit(info, 101), undefined);
});
test("Claude keeps its sources and includes scoped windows", () => {
  const info = adaptClaude({ source: "statusline", fetched_at: 100, limits: [
    { kind: "weekly_scoped", label: "Fable", percent: 80, severity: "warning", resets_at: 3000 }] });
  assert.equal(info.source, "statusline"); assert.equal(summaryLimit(info, 101).label, "Fable 7d");
});

test("Claude preserves the existing over-limit severity fallback", () => {
  const info = adaptClaude({ source: "oauth", fetched_at: 100, limits: [
    { kind: "session", label: "Session", percent: 100, severity: "exceeded", resets_at: 3000 }] });
  assert.equal(info.limits[0].severity, "over");
});

test("strip windows are all current limits in stable duration and bucket order", () => {
  const info = adaptCodex({ ...wire, limits: [limit("primary", 10080, 24, 3000),
    limit("secondary", 300, 21, 3000), limit("primary", 10080, 99, 3000, "reserve"),
    limit("primary", 60, 90, 99, "expired")] });
  assert.deepEqual(model.stripLimits(info, 100).map(l => l.id), ["codex:secondary", "codex:primary", "reserve:primary"]);
  assert.equal(summaryLimit(info, 100).percent, 24, "reserve does not replace the main allowance summary");
  assert.deepEqual(model.stripLimits({ ...info, limits: [...info.limits].reverse() }, 100), model.stripLimits(info, 100));
  const claude = adaptClaude({ source: "oauth", fetched_at: 90, limits: [
    {kind:"weekly_scoped",label:"Fable",percent:80,resets_at:3000,severity:"warning"},
    {kind:"weekly_all",label:"Weekly",percent:59,resets_at:3000,severity:"normal"},
    {kind:"session",label:"Session",percent:32,resets_at:null,severity:"normal"},
    {kind:"weekly_scoped",label:"Past reset",percent:99,resets_at:99,severity:"warning"}] });
  assert.deepEqual(model.stripLimits(claude, 100).map(l => l.label), ["5h", "7d", "Fable 7d", "Past reset 7d"]);
  assert.equal(summaryLimit({ ...claude, limits: model.stripLimits(claude, 100) }, 100).id,
    summaryLimit(claude, 100).id, "strip and accessible summary keep the same Claude window after reset");
});

// Render the actual row component, including its countdown formatter. A copy of
// the display rule would miss regressions in the JSX branch itself.
const rowsSource = app.slice(app.indexOf("function UsageRows("), app.indexOf("function UsageMeter("));
const etaSource = app.slice(app.indexOf("function fmtEta("), app.indexOf("/** Is anyone actually looking"));
const rowJs = (await transformWithEsbuild(etaSource + rowsSource, "rows.tsx", { loader: "tsx" })).code;
const UsageRows = new Function("React", "Fragment", "expired", `${rowJs}; return UsageRows;`)(React, Fragment, expired);
test("Claude known resets in the past are not unknown", () => {
  const row = reset => renderToStaticMarkup(React.createElement(UsageRows, { nowSec: 100,
    info: adaptClaude({ source: "oauth", fetched_at: 90, limits: [
      { kind: "session", label: "Session", percent: 20, severity: "normal", resets_at: reset }] }) }));
  for (const reset of [99, 100]) assert(!row(reset).includes("Reset unknown"));
  assert(row(null).includes("Reset unknown"));
  assert(row(200).includes("in 1m"));
});
// From the dispatch table the hook reads (`USAGE_READ`) through the hooks, so
// the check runs the real per-harness dispatch rather than a stand-in for it.
const from = app.indexOf("const USAGE_READ"); const to = app.indexOf("function UsageRows(", from);
assert(from >= 0 && to > from && app.indexOf("function useProviderUsage(", from) > from && app.indexOf("function useProviderUsage(", from) < to);
const hooks = (await transformWithEsbuild(app.slice(from, to), "hooks.ts", { loader: "ts" })).code;
function host() {
  const states = [], effects = [], pending = [], requests = [], timers = new Map(), listeners = new Map();
  const document = { visibilityState: "visible" }; let cursor = 0, timerId = 0;
  const useState = initial => {
    const i = cursor++;
    if (!(i in states)) states[i] = typeof initial === "function" ? initial() : initial;
    return [states[i], next => { states[i] = typeof next === "function" ? next(states[i]) : next; }];
  };
  const useEffect = (fn, deps) => {
    const i = cursor++; const old = effects[i];
    if (!old || deps.some((d, n) => d !== old.deps[n])) pending.push(() => {
      old?.cleanup?.(); effects[i] = { deps, cleanup: fn() };
    });
  };
  const invoke = command => new Promise((resolve, reject) => requests.push({ command, resolve, reject }));
  const window = { addEventListener: (e, f) => { const s = listeners.get(e) ?? new Set(); s.add(f); listeners.set(e, s); },
    removeEventListener: (e, f) => listeners.get(e)?.delete(f) };
  const setInterval = (fn, delay) => { timers.set(++timerId, { fn, delay }); return timerId; };
  const clearInterval = id => timers.delete(id);
  const useUsage = new Function("useState", "useEffect", "invoke", "document", "window", "setInterval", "clearInterval",
    "checking", "adaptClaude", "adaptCodex", "adaptKimi", "viewUsage", `const USAGE_POLL_MS=180000, USAGE_TICK_MS=${USAGE_TICK_MS}; ${hooks}; return useUsage;`)(
    useState, useEffect, invoke, document, window, setInterval, clearInterval, checking, adaptClaude, adaptCodex, adaptKimi, viewUsage);
  const onError = () => {};
  // `kimi` last and off by default: the tests above it count Claude's and
  // Codex's requests only.
  const render = (enabled = true, claude = true, codex = true, visible = true, kimi = false) => {
    cursor = 0; const result = useUsage(enabled, claude, codex, kimi, visible, onError);
    while (pending.length) pending.shift()(); return result;
  };
  return { render, requests, timers, document, providerState: provider => states.find(s => s?.provider === provider), focus: () => listeners.get("focus")?.forEach(f => f()),
    unmount: () => effects.forEach(e => e?.cleanup?.()) };
}
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
test("global off and independent toggles suppress commands", () => {
  const h = host(); assert.equal(h.render(false).info, null); assert.equal(h.requests.length, 0);
  h.render(true, false, true); assert.deepEqual(h.requests.map(r => r.command), ["codex_usage"]);
  assert.equal(h.render(true, false, false).info, null); assert.equal(h.timers.size, 0); h.unmount();
});
test("the kimi toggle alone asks kimi_usage, and its answer is adapted", async () => {
  const h = host(); h.render(true, false, false, true, true);
  assert.deepEqual(h.requests.map(r => r.command), ["kimi_usage"]);
  h.requests[0].resolve({ ...kimiWire, fetched_at: Math.floor(Date.now() / 1000), limits: kimiWire.limits.map(l => ({ ...l, resets_at: null })) });
  await flush();
  const info = h.render(true, false, false, true, true).info;
  assert.equal(info[0].provider, "kimi"); assert.deepEqual(info[0].limits.map(l => l.label), ["5h", "month"]);
  h.unmount();
});
test("hidden startup, transition and focus do not invoke", () => {
  const h = host(); h.document.visibilityState = "hidden"; h.render(true, true, true, false); h.focus();
  assert.equal(h.requests.length, 0); assert.equal(h.timers.size, 0);
  h.document.visibilityState = "visible"; h.render(); assert.equal(h.requests.length, 2);
  h.document.visibilityState = "hidden"; h.render(true, true, true, false); h.focus();
  assert.equal(h.requests.length, 2); assert.equal(h.timers.size, 0); h.unmount();
});
{
  const h = host(); h.render();
  h.requests[0].resolve({ source: "oauth", fetched_at: Math.floor(Date.now()/1000), limits: [] }); await flush();
  test("slow Codex does not block Claude", () => {
    const value = h.render().info; assert.equal(value[0].state, "unavailable"); assert.equal(value[1].state, "checking");
  });
  h.focus();
  h.requests[3].resolve({ ...wire, fetched_at: Math.floor(Date.now()/1000) }); await flush();
  h.requests[1].reject(new Error("late failure")); await flush();
  test("older failure cannot replace newer success", () => assert.notEqual(h.render().info[1].state, "unavailable"));
  h.focus();
  const pendingCodex = h.requests.at(-1);
  assert.equal(pendingCodex.command, "codex_usage");
  h.render(true, true, false);
  assert.equal(h.providerState("codex").state, "checking");
  pendingCodex.resolve({ ...wire, fetched_at: Math.floor(Date.now()/1000) }); await flush();
  test("disabled Codex reply cannot repopulate its state", () => {
    assert.equal(h.providerState("codex").state, "checking");
    assert.equal(h.render(true, true, false).info.length, 1);
  });
  h.unmount();
}
assert.match(app, /useUsage\(settingsReady && settings\.usage_place !== "off"/,
  "saved Off must gate startup, before settings hydration");

console.log(`plan-usage-check: ${checks} passed`);
