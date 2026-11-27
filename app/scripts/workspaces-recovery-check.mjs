// Exercise App's actual workspace-loading effects under a refused read.
// Usage: node app/scripts/workspaces-recovery-check.mjs [App.tsx]
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { transformWithEsbuild } from 'vite';
import assert from 'node:assert/strict';
const file = process.argv[2] ?? fileURLToPath(new URL('../src/App.tsx', import.meta.url));
const app = fs.readFileSync(file, 'utf8');
const start = app.indexOf('  const wsRegKey =');
const end = app.indexOf('  /** Adding never asks.', start);
assert(start >= 0 && end > start);
const code = `export function render(env) {
const {ws, wsList, invoke, setWsList, useEffect, window, remoteHost} = env;
type WsList = any; const WS_CHANGED_KEY = 'ws-changed';
${app.slice(start, end)}
}`;
const { code: js } = await transformWithEsbuild(code, 'workspace-effects.ts', { loader: 'ts', format: 'esm' });
const { render } = await import('data:text/javascript;base64,' + Buffer.from(js).toString('base64'));
const list = { workspaces: [{ id: 'a', name: 'Kept workspace', projects: [] }] };
const flush = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
function harness(remote = null) {
  let state = null, roots = [], slots = [], next = 0, calls = [], listeners = new Map(), timers = new Map(), timerId = 0;
  const window = {
    addEventListener: (k, f) => { if (!listeners.has(k)) listeners.set(k, new Set()); listeners.get(k).add(f); },
    removeEventListener: (k, f) => listeners.get(k)?.delete(f),
    setTimeout: (f) => { timers.set(++timerId, f); return timerId; },
    clearTimeout: (id) => timers.delete(id),
  };
  const env = { window, remoteHost: () => remote,
    invoke: (cmd) => { assert.equal(cmd, 'workspaces_list'); return new Promise((resolve, reject) => calls.push({ resolve, reject })); },
    setWsList: (v) => { state = typeof v === 'function' ? v(state) : v; },
    useEffect: (effect, deps) => {
      const i = next++, old = slots[i];
      if (old && deps.every((v,j) => Object.is(v, old.deps[j]))) return;
      old?.cleanup?.(); slots[i] = { deps, cleanup: effect() };
    },
  };
  const draw = () => { next = 0; render({ ...env, ws: { projects: roots.map(root => ({root})) }, wsList: state }); };
  return { draw, calls, get state() { return state; }, roots: (r) => { roots = r; draw(); },
    focus: () => { for (const f of listeners.get('focus') ?? []) f(); },
    tick: () => { const todo = [...timers.values()]; timers.clear(); todo.forEach(f => f()); },
    stop: () => slots.forEach(s => s.cleanup?.()), get timers() { return timers.size; } };
}
let failed = 0;
async function test(name, fn) { try { await fn(); console.log('ok — ' + name); } catch(e) { failed++; console.log('not ok — ' + name + '\n' + e.message); } }
await test('registry refresh refused busy keeps workspace headers and focus can refresh', async () => {
 const h = harness(); h.draw(); h.calls.shift().resolve(list); await flush(); h.draw();
 h.roots(['/old', '/new-unassigned']); h.calls.shift().reject('busy'); await flush(); h.draw();
 assert.deepEqual(h.state, list, 'a transient refusal must retain the loaded workspace list');
 h.focus(); assert.equal(h.calls.length, 1); h.calls.shift().resolve({workspaces: []}); await flush();
 assert.deepEqual(h.state, {workspaces: []}, 'a successful empty list is still authoritative'); h.stop();
});
await test('first read refused busy recovers on focus without another registry edit', async () => {
 const h = harness(); h.draw(); h.calls.shift().reject('busy'); await flush(); h.draw();
 h.focus(); assert.equal(h.calls.length, 1, 'focus must still listen while the list is unavailable');
 h.calls.shift().resolve(list); await flush(); assert.deepEqual(h.state, list); h.stop();
});
await test('busy retries without focus, and a replaced effect cannot clear a newer answer', async () => {
 const h = harness(); h.draw(); const stale = h.calls.shift();
 h.roots(['/new']); h.calls.shift().resolve(list); await flush(); stale.reject('busy'); await flush();
 assert.deepEqual(h.state, list); h.roots(['/new','/another']); h.calls.shift().reject('busy'); await flush();
 h.tick(); assert.equal(h.calls.length, 1, 'a failed read schedules a retry');
 h.calls.shift().resolve(list); await flush(); h.stop(); assert.equal(h.timers, 0);
});
await test('focus coalesces reads, and unmount cancels recovery and ignores late answers', async () => {
 const h = harness(); h.draw(); h.focus(); h.focus(); assert.equal(h.calls.length, 1);
 const pending = h.calls.shift(); h.stop(); pending.resolve(list); await flush();
 assert.equal(h.state, null); h.focus(); h.tick(); assert.equal(h.calls.length, 0);
 const retry = harness(); retry.draw(); retry.calls.shift().reject('busy'); await flush();
 assert.equal(retry.timers, 1); retry.stop(); assert.equal(retry.timers, 0);
});
await test('remote windows neither read the console list nor schedule retries', async () => {
 const h = harness('remote'); h.draw(); h.focus(); h.tick();
 assert.equal(h.calls.length, 0); assert.equal(h.state, null); assert.equal(h.timers, 0); h.stop();
});
process.exitCode = failed ? 1 : 0;
