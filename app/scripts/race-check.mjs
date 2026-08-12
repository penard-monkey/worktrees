// Deterministic check of App.tsx's refresh/commitWs/patchDeclared/mutate under
// controlled promise-resolution orders.
//
// It does NOT paraphrase the app: it slices the REAL source text out of
// App.tsx between stable markers and evaluates it with React/Tauri stubs, so
// running it before and after an edit tests the edit itself.
//
//   node race-check.mjs [path/to/App.tsx]
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// Type-stripping via vite's own esbuild re-export: vite is a direct dependency
// and resolves from app/node_modules, whereas bare "esbuild" does not under
// pnpm's strict layout (it is only a transitive dep, reachable via .pnpm).
import { transformWithEsbuild } from "vite";

const APP = process.argv[2] || fileURLToPath(new URL("../src/App.tsx", import.meta.url));
const lines = fs.readFileSync(APP, "utf8").split("\n");

// Slice [first line containing `from`, first later line containing `to`) — the
// markers are statements that bracket the block, so the block may change shape.
function cut(from, to, required = true) {
  const a = lines.findIndex((l) => l.includes(from));
  if (a < 0) { if (required) throw new Error("marker not found: " + from); return ""; }
  const b = lines.findIndex((l, i) => i > a && l.includes(to));
  if (b < 0) throw new Error("end marker not found: " + to);
  return lines.slice(a, b).join("\n");
}
const REFRESH = cut('const lastSnap = useRef("");', "useEffect(() => { refresh(); }");
const PATCH = cut("const patchDeclared =", "const mutate = async", false);
const MUTATE = cut("const mutate = async", "// Returns the op's CmdResult");
const has = (s) => REFRESH.includes(s) || PATCH.includes(s);
console.log(`[${APP.split("/").pop()}] refresh+commitWs ${REFRESH.split("\n").length}L, `
  + `patchDeclared ${PATCH ? PATCH.split("\n").length + "L" : "ABSENT"}, mutate ${MUTATE.split("\n").length}L`);

const src = `
type Workspace = any; type Declared = any; type Place = any;
export function build(env: any) {
  const { useRef, useCallback, invoke, setWs, setErr, fail } = env;
  const refreshErr = useRef("");
${REFRESH}
${PATCH}
${MUTATE}
  return { refresh, commitWs, patchDeclared: ${PATCH ? "patchDeclared" : "null"}, mutate, lastSnap };
}`;
const js = (await transformWithEsbuild(src, "race-check.ts", { loader: "ts", format: "esm" })).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── harness ──────────────────────────────────────────────────────────────────
const REPO = "/repo", SLUG = "feedback-20260804";
// nameOf() from App.tsx:111 — what the nav row actually renders.
const nameOf = (w) => {
  const p = w?.projects?.[0]?.snapshot?.places?.[0];
  return p?.declared?.title?.trim() || p?.slug || "<none>";
};

function harness() {
  const store = {};                       // .worktrees.places.json, backend side
  let ws = null, renders = 0;
  const inflight = [];                    // unresolved list_workspace reads
  const env = {
    useRef: (v) => ({ current: v }),
    useCallback: (f) => f,
    setErr: () => {},
    fail: () => {},
    setWs: (x) => { ws = typeof x === "function" ? x(ws) : x; renders++; },
    invoke: (cmd, args) => {
      if (cmd === "list_workspace") {
        // The git sweep READS here; the promise settles when the test says so.
        const w = { projects: [{ root: REPO, ok: true, snapshot: {
          places: [{ slug: SLUG, dirty: false, declared: { ...store } }] } }] };
        let done; const p = new Promise((r) => (done = r));
        inflight.push({ settle: () => done(w) });
        return p;
      }
      if (cmd === "set_title") {
        if (args.fail) return Promise.reject("store: read-only file system");
        const t = args.title.trim();
        if (t) store.title = t; else delete store.title;   // lib.rs set_title
        return Promise.resolve();
      }
      if (cmd === "log_event") return Promise.resolve();
      throw new Error("unmocked invoke: " + cmd);
    },
  };
  const api = build(env);
  const h = {
    api, env, store, inflight,
    nav: () => nameOf(ws),
    renders: () => renders,
    mark: () => (renders = 0),
    // setTitle() from App.tsx, verbatim in behaviour
    setTitle: (title, opts = {}) => {
      if (api.patchDeclared) api.patchDeclared(REPO, SLUG, { title: title.trim() || undefined });
      return api.mutate(env.invoke("set_title", { repo: REPO, slug: SLUG, title, ...opts }));
    },
    take: () => inflight.shift(),   // oldest in-flight read
    last: () => inflight.pop(),     // newest in-flight read
  };
  return h;
}
const tick = () => new Promise((r) => setImmediate(r));
const settle = async () => { for (let i = 0; i < 50; i++) await tick(); };
const prime = async (h) => { h.api.refresh(); await tick(); h.take().settle(); await settle(); h.mark(); };
// Run a rename to completion: settle every read it starts (mutate awaits its own).
const rename = async (h, title, opts) => {
  const p = h.setTitle(title, opts);
  await settle();
  while (h.inflight.length) { h.take().settle(); await settle(); }
  await p;
};

const out = [];
const check = (name, got, want) => {
  const ok = got === want;
  out.push({ name, ok, got, want });
  console.log(`  ${ok ? "PASS" : "FAIL"}  ${name}  (got ${JSON.stringify(got)}, want ${JSON.stringify(want)})`);
};

// [1] THE REPORTED BUG: a list_workspace that STARTED BEFORE the store write
//     RESOLVES AFTER the mutate's own refresh, and puts the old name back.
async function staleReadWinsRace(optimistic) {
  const h = harness();
  await prime(h);
  h.api.refresh(); await tick();          // P: poll refresh, reads the PRE-write store
  const P = h.last();
  // optimistic=false is the v0.12.0 shape: mutate + refresh, no patchDeclared.
  const m = optimistic && h.api.patchDeclared
    ? h.setTitle("asset-hub")
    : h.api.mutate(h.env.invoke("set_title", { repo: REPO, slug: SLUG, title: "asset-hub" }));
  await settle();                          // write lands; mutate starts refresh M
  const M = h.last();
  M.settle(); await settle();              // M resolves FIRST (post-write snapshot)
  const mid = h.nav();
  P.settle(); await settle(); await m;     // P resolves LAST (pre-write snapshot)
  return { mid, end: h.nav(), h };
}

// [2] A declared write that FAILS must not leave its optimistic value on screen.
//     The confirming refresh returns a workspace byte-identical to `lastSnap`,
//     so the dedupe bails and never calls setWs.
async function failedWriteStickiness() {
  const h = harness();
  h.store.title = "asset-hub";
  await prime(h);
  await rename(h, "typo-name", { fail: true });
  return h.nav();
}

// [3] The battery dedupe: idle polls returning a byte-identical workspace must
//     not re-render the tree. (The reason the byte compare exists at all.)
async function idlePollDedupe() {
  const h = harness();
  await prime(h);
  for (let i = 0; i < 10; i++) { h.api.refresh(); await tick(); h.take().settle(); await settle(); }
  return h.renders();
}

// [3b] ...including two idle polls that OVERLAP and resolve out of order.
async function overlappingIdleDedupe() {
  const h = harness();
  await prime(h);
  h.api.refresh(); await tick();
  h.api.refresh(); await tick();
  h.last().settle(); await settle();
  h.take().settle(); await settle();
  return h.renders();
}

// [3c] ...and after a rename has settled, the poll must go quiet again.
async function dedupeAfterRename() {
  const h = harness();
  await prime(h);
  await rename(h, "asset-hub");
  h.mark();
  for (let i = 0; i < 10; i++) { h.api.refresh(); await tick(); h.take().settle(); await settle(); }
  return h.renders();
}

// [4] An out-of-band change (CLI, other window) must still reach the nav.
async function outOfBandChange() {
  const h = harness();
  await prime(h);
  await rename(h, "asset-hub");
  h.store.title = "from-cli";
  h.api.refresh(); await tick(); h.take().settle(); await settle();
  return h.nav();
}

// [5] LIVENESS: reads that overlap continuously (each sweep outlasts the next
//     trigger). Every read still carries fresher data than the one before it,
//     so the nav must track the store — a guard that only lets the NEWEST-
//     STARTED read speak drops all of them and freezes the tree.
async function pipelinedReads() {
  const h = harness();
  await prime(h);
  h.store.title = "asset-hub";        // out-of-band change (CLI / other window)
  h.api.refresh(); await tick();      // R1 reads it
  for (let i = 0; i < 5; i++) {
    h.api.refresh(); await tick();    // a newer read starts...
    h.take().settle(); await settle(); // ...before the oldest one resolves
  }
  return h.nav();
}

// [6] commitWs() has the same exposure: add_project / remove_project /
//     init_repo / create_initial_commit hand it a workspace the backend just
//     built, but a read that STARTED EARLIER can still resolve afterwards and
//     put the pre-command workspace back.
async function commitWsRace() {
  const h = harness();
  await prime(h);
  h.api.refresh(); await tick();          // R reads the PRE-command workspace
  const R = h.last();
  h.store.title = "asset-hub";            // the command changed the backend
  h.api.commitWs({ projects: [{ root: REPO, ok: true, snapshot: {
    places: [{ slug: SLUG, dirty: false, declared: { title: "asset-hub" } }] } }] });
  R.settle(); await settle();
  return h.nav();
}

console.log("\n[1] rename while an earlier list_workspace is still in flight");
const r1 = await staleReadWinsRace(true);
check("nav right after the mutate's own refresh", r1.mid, "asset-hub");
check("nav after the stale pre-write read resolves", r1.end, "asset-hub");

console.log("\n[2] declared write FAILS -> optimistic value must be undone");
check("nav after the confirming refresh", await failedWriteStickiness(), "asset-hub");

console.log("\n[3] the battery dedupe must survive");
check("re-renders from 10 idle polls", await idlePollDedupe(), 0);
check("re-renders from 2 overlapping idle polls", await overlappingIdleDedupe(), 0);
check("re-renders from 10 idle polls after a rename", await dedupeAfterRename(), 0);

console.log("\n[5] LIVENESS: continuously overlapping reads must not freeze the nav");
check("nav after 5 pipelined reads", await pipelinedReads(), "asset-hub");

console.log("\n[6] commitWs must also outrank a read already in flight");
check("nav after the pre-command read resolves", await commitWsRace(), "asset-hub");

console.log("\n[4] an out-of-band change still lands");
check("nav after the store changes underneath", await outOfBandChange(), "from-cli");

const bad = out.filter((r) => !r.ok);
console.log(`\n${bad.length ? bad.length + " FAILING: " + bad.map((b) => b.name).join("; ") : "ALL GREEN"}`);
process.exit(bad.length ? 1 : 0);
