// Behavioral adapter/config tests run with the real TypeScript, not a mirror.
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import vm from 'node:vm';
import ts from '../node_modules/typescript/lib/typescript.js';

function evaluate(path, context = {}, rewrite = s => s) {
  const source = rewrite(fs.readFileSync(path, 'utf8'));
  const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports = {};
  vm.runInNewContext(js, { exports, URL, console, ...context }, { filename: path });
  return exports;
}
const { feedbackConfig } = evaluate('app/src/feedbackConfig.ts');
for (const [key, url, dev] of [
  ['', 'https://example.test', false], ['pk_test_key', '', false],
  ['pk_test_key', 'http://example.test', true], ['pk_test_key', 'http://localhost:4100', false],
  ['pk_test_key', 'https://user:pass@example.test', false],
  ['pk_test_key', 'https://example.test/path', false], ['pk_test_key', 'https://example.test?q=x', false],
  ['pk_test_key', 'https://example.test#x', false], ['pk_test_key', 'javascript:alert(1)', true],
]) assert.equal(feedbackConfig(key, url, dev), null, `refuse ${url}`);
assert.equal(feedbackConfig('pk_test_key', 'https://example.test/', false)?.apiUrl, 'https://example.test');
assert.equal(feedbackConfig('pk_test_key', 'http://localhost:4100', true)?.apiUrl, 'http://localhost:4100');

const events = new Map();
const window = { addEventListener: (k, f) => events.set(k, f), removeEventListener: k => events.delete(k) };
const escape = evaluate('app/src/useEscape.ts', { window, require: () => ({}) });
let settingsClosed = 0, feedbackClosed = 0;
const lower = escape.registerEscape(() => settingsClosed++);
const upper = escape.registerEscape(() => feedbackClosed++);
const press = () => events.get('keydown')({ key: 'Escape', preventDefault() {}, stopPropagation() {} });
press();
assert.equal(settingsClosed, 0);
assert.equal(feedbackClosed, 1);
upper(); upper();
press();
assert.equal(settingsClosed, 1);
lower();
assert.equal(escape.escapeDepth(), 0);
assert.equal(events.size, 0);

async function adapter({ mock = false, configured = true, label = 'main', rejectVersion = false } = {}) {
  let sdk = 0, fake = 0, destroyed = 0, calls = [], options;
  let resolveVersion;
  const version = rejectVersion ? Promise.reject(Error('private native details')) : new Promise(r => { resolveVersion = r; });
  // Prevent an unhandled rejection when no configuration means it is never read.
  version.catch(() => {});
  const widget = { open() {}, close() {}, destroy() { destroyed++; } };
  const env = { VITE_MOCK: mock ? '1' : '', DEV: true, VITE_ORFIS_KEY: configured ? 'pk_test_key' : '', VITE_ORFIS_API_URL: 'http://localhost:4100' };
  const mod = evaluate('app/src/feedback.ts', {
    require(name) {
      if (name === '@tauri-apps/api/core') return { invoke: (cmd, arg) => { calls.push([cmd, arg]); return cmd === 'get_changelog' ? version : Promise.resolve(); } };
      if (name === '@tauri-apps/api/window') return { getCurrentWindow: () => ({ label }) };
      if (name === './feedbackConfig') return { feedbackConfig };
      if (name === './useEscape') return { registerEscape: escape.registerEscape };
      if (name === './feedbackWidget') { sdk++; return { createFeedbackWidget: (_config, appVersion, cb) => { options = { appVersion, cb }; return widget; } }; }
      if (name === './mock/feedback') { fake++; return { createMockFeedback: cb => { options = { cb }; return widget; } }; }
      throw Error(name);
    },
  }, s => s.replaceAll('import.meta.env', JSON.stringify(env)));
  return { mod, resolveVersion, counts: () => ({ sdk, fake, destroyed }), calls, options: () => options };
}
for (const args of [{ configured: false }, { label: 'docs-123' }]) {
  const a = await adapter(args);
  await a.mod.startFeedback();
  assert.deepEqual(a.counts(), { sdk: 0, fake: 0, destroyed: 0 });
  assert.equal(a.calls.length, 0);
  assert.equal(a.mod.feedbackSnapshot().available, false);
}
const real = await adapter();
const first = real.mod.startFeedback();
assert.equal(real.mod.startFeedback(), first, 'concurrent startup shares promise');
real.resolveVersion({ version: '9.8.7' });
await first;
await real.mod.startFeedback();
assert.equal(real.counts().sdk, 1);
assert.equal(real.options().appVersion, '9.8.7');
assert.equal(real.mod.feedbackSnapshot().available, true);
// Both queue outcomes, because only one of them is safe to quit on. The SDK
// reports this truthfully as of Orfis 3222a74; before that a shed queue called
// back as plain success, so the notice could promise a retry for a report that
// no longer existed.
real.options().cb.queued({ durable: true });
assert.match(real.mod.feedbackSnapshot().notice, /not yet received/);
assert.doesNotMatch(real.mod.feedbackSnapshot().notice, /memory only/, 'a durable queue must not warn about losing it');
real.options().cb.queued({ durable: false });
assert.match(real.mod.feedbackSnapshot().notice, /memory only/, 'a memory-only queue must say so');
real.options().cb.accepted();
assert.match(real.mod.feedbackSnapshot().notice, /received your feedback/);
real.mod.stopFeedback();
assert.equal(real.counts().destroyed, 1);
const late = await adapter();
const pending = late.mod.startFeedback();
late.mod.stopFeedback();
late.resolveVersion({ version: '9.8.7' });
await pending;
assert.equal(late.counts().sdk, 1, 'module import can finish, but must not create widget');
assert.equal(late.options(), undefined);
assert.equal(late.mod.feedbackSnapshot().available, false);
const mock = await adapter({ mock: true });
await mock.mod.startFeedback();
assert.equal(mock.counts().sdk, 0, 'mock must never even import SDK');
assert.equal(mock.counts().fake, 1);
assert.equal(mock.calls.length, 0);
const bad = await adapter({ rejectVersion: true });
await bad.mod.startFeedback();
assert.equal(bad.counts().sdk, 0);
assert.equal(bad.mod.feedbackSnapshot().available, false);
assert.equal(bad.calls.at(-1)[1].msg, 'Feedback initialization failed');
// The real SDK path stays shut until packaged acceptance. Asserted, not merely
// commented: flipping the constant has to be a deliberate act that also updates
// this line, which is the "credentials alone must not enable it" rule with
// teeth.
const widgetSrc = fs.readFileSync('app/src/feedbackWidget.ts', 'utf8');
assert.match(widgetSrc, /const ACCEPTANCE_PENDING = true;/, 'the real SDK path must stay gated');
assert.match(widgetSrc, /captureDiagnostics: false/);
assert.match(widgetSrc, /collectDeviceContext: false/);
assert.match(widgetSrc, /askForEmail: false/);

// We ship someone else's bytes, so the terms have to ship WITH them. Two
// independent carriers, because each fails differently: the sidecar file is
// what a reader looks for, and the in-artifact banner is what survives the
// bundle being copied somewhere on its own. A re-vendor from a revision that
// predates the license (or a build config that drops `postBanner`, which Vite
// does silently in library mode) goes red here rather than shipping unlicensed.
const bundle = 'app/src/vendor/orfis/orfis.es.js';
assert.ok(fs.existsSync('app/src/vendor/orfis/LICENSE'), 'the vendored SDK must ship its LICENSE');
assert.match(
  fs.readFileSync('app/src/vendor/orfis/LICENSE', 'utf8'),
  /^MIT License/,
  'the vendored LICENSE must be the MIT text',
);
assert.match(
  fs.readFileSync(bundle, 'utf8').split('\n', 1)[0],
  /^\/\*! Orfis web SDK \| MIT License \| Copyright \(c\) \d{4} /,
  'the vendored bundle must carry its license notice on line 1',
);
// And the provenance README must describe the artifact that is actually here —
// a re-vendor that updates the bytes and forgets the hash is the whole reason
// this file is reviewed at all.
const sha = crypto.createHash('sha256').update(fs.readFileSync(bundle)).digest('hex');
assert.match(
  fs.readFileSync('app/src/vendor/orfis/README.md', 'utf8'),
  new RegExp(`SHA-256: \`${sha}\``),
  `README must record the bundle's real SHA-256 (${sha})`,
);

// Routing lives in vite's two MODE files, and the invariant that matters is
// not which key is in them — it is that the laptop endpoint can NEVER ship.
// Asserted with a dummy key so this holds both before the real keys are pasted
// and after: the URL rule is what is under test, not the credential.
const DUMMY = 'pk_test_key';
const envRouting = (file) => Object.fromEntries(
  fs.readFileSync(file, 'utf8').split('\n')
    .filter((l) => l.startsWith('VITE_ORFIS_'))
    .map((l) => [l.slice(0, l.indexOf('=')), l.slice(l.indexOf('=') + 1)]),
);
for (const f of ['app/.env.production', 'app/.env.development']) {
  assert.ok(fs.existsSync(f), `${f} must exist — routing is release-owned, not user-supplied`);
}
const prodUrl = envRouting('app/.env.production').VITE_ORFIS_API_URL;
const devUrl = envRouting('app/.env.development').VITE_ORFIS_API_URL;
// A release build must accept the production endpoint...
assert.equal(
  feedbackConfig(DUMMY, prodUrl, false)?.apiUrl, prodUrl,
  'the production endpoint must be usable in a release build',
);
// ...and must REFUSE the development one. This is the whole point of splitting
// the files by mode: `feedbackConfig` permits http:// only for a loopback host
// in a dev build, so the laptop endpoint cannot reach a shipped binary even if
// the file is copied there.
assert.equal(
  feedbackConfig(DUMMY, devUrl, false), null,
  'the development endpoint must be refused in a release build',
);
assert.equal(
  feedbackConfig(DUMMY, devUrl, true)?.apiUrl, devUrl,
  'the development endpoint must work in a dev build',
);

console.log('ok — feedback config, singleton, main-window boundary, offline mock, version, notices (durable + memory), teardown, Escape, SDK gate, vendored license + recorded hash, mode-split routing');
