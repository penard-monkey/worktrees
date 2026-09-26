// Exercise the real normalization and polling hook with a virtual scheduler.
import fs from "node:fs";
import assert from "node:assert/strict";
import React, { Fragment } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { transformWithEsbuild } from "vite";
const src = fs.readFileSync(new URL("../src/planUsage.ts", import.meta.url), "utf8");
const js = (await transformWithEsbuild(src, "planUsage.ts", { loader: "ts" })).code;
const model = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
const { adaptCodex, adaptClaude, viewUsage, summaryLimit, windowLabel, checking, expired, CODEX_STALE_SECS, compactUsage } = model;
let checks = 0;
function test(name, fn) { fn(); checks++; console.log(`ok ${name}`); }
const limit = (role, minutes, percent, reset, bucket = "codex") => ({ id: `${bucket}:${role}`, bucket_id: bucket,
  bucket_label: bucket, window_role: role, window_minutes: minutes, percent, severity: "normal", resets_at: reset });
const wire = { state: "ready", source: "app_server", fetched_at: 100, reason: null,
  limits: [limit("primary", 10080, 24, 110), limit("secondary", null, 21, 3000)] };
const rust = fs.readFileSync(new URL("../src-tauri/src/codex_usage.rs", import.meta.url), "utf8").split("#[cfg(test)]")[0];
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
test("age ceiling is inclusive and future timestamps are unavailable", () => {
  assert.equal(viewUsage(adaptCodex(wire), 100 + CODEX_STALE_SECS).limits.length, 2);
  assert.equal(viewUsage(adaptCodex(wire), 101 + CODEX_STALE_SECS).limits.length, 0);
  assert.equal(viewUsage(adaptCodex(wire), 99).state, "unavailable");
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

const app = fs.readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
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
const from = app.indexOf("function useProviderUsage("); const to = app.indexOf("function UsageRows(", from);
assert(from >= 0 && to > from);
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
    "checking", "adaptClaude", "adaptCodex", "viewUsage", `const USAGE_POLL_MS=180000, USAGE_TICK_MS=15000; ${hooks}; return useUsage;`)(
    useState, useEffect, invoke, document, window, setInterval, clearInterval, checking, adaptClaude, adaptCodex, viewUsage);
  const onError = () => {};
  const render = (enabled = true, claude = true, codex = true, visible = true) => {
    cursor = 0; const result = useUsage(enabled, claude, codex, visible, onError);
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
