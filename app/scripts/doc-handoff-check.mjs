// The hand-off of a document between the windows (app/src/doc/handoff.ts), evaluated
// on a virtual clock with two windows over one scripted store.
//   node app/scripts/doc-handoff-check.mjs        (from the repo root)
import { readFileSync } from "node:fs";
import { transformWithEsbuild } from "vite";
const src = readFileSync(new URL("../src/doc/handoff.ts", import.meta.url), "utf8");
const js = (await transformWithEsbuild(src, "handoff.ts", { loader: "ts" })).code;
const H = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
let failed = 0;
const eq = (m, got, want) => { if (JSON.stringify(got) !== JSON.stringify(want)) { failed++; console.log(`not ok — ${m}\n     got: ${JSON.stringify(got)}\n    want: ${JSON.stringify(want)}`); } else console.log(`ok   ${m}`); };
const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); };

const A = "01JC8Z4K3M9QX2T7VB00000901", B = "01JC8Z4K3M9QX2T7VB00000902", W = "01JC8Z4K3M9QX2T7VB00000903";
const idea = { kind: "idea", id: A }, mem = { kind: "memory", ws: W, path: "topics/nas.md", name: "Jellyfin" };
function rig() {
  const store = new Map(), ears = [], ticks = new Set();
  let now = 1_000_000, n = 0, broken = false;
  const env = {
    get: (k) => { if (broken) throw new Error("SecurityError"); return store.get(k) ?? null; },
    set: (k, v) => { if (broken) throw new Error("QuotaExceeded"); store.set(k, v); for (const [key, fn] of [...ears]) if (key === k) fn(); },
    remove: (k) => store.delete(k),
    heard: (k, fn) => { const e = [k, fn]; ears.push(e); return () => ears.splice(ears.indexOf(e), 1); },
    every: (_, fn) => { ticks.add(fn); return () => ticks.delete(fn); },
    now: () => now, id: () => `ask-${++n}`,
  };
  return { env, store, ears, ticks, breaks: () => { broken = true; }, async tick(ms) { now += ms; for (const f of [...ticks]) f(); await settle(); } };
}
/** An ask, and what it came to (`undefined`: still out). */
function go(r, doc, raise = async () => {}, way = H.FLOAT) { const o = { out: undefined }; void H.ask(r.env, way, doc, raise, "The Ideas window").then((x) => { o.out = x; }); return o; }

for (const doc of [idea, mem]) {
  const r = rig(), o = go(r, doc);
  await settle();
  eq(`${doc.kind}: asked, nothing is over until it answers`, [o.out, H.readAsk(r.store.get(H.FLOAT.ask), r.env.now())?.doc], [undefined, doc]);
  const a = H.take(r.env, H.FLOAT);
  eq("…taken once", [a.doc, H.take(r.env, H.FLOAT)], [doc, null]);
  H.answer(r.env, H.FLOAT, a, { ok: true, doc });
  await settle();
  eq("…an answer that names it: it moved, and nothing listens any more", [o.out, r.ears.length, r.ticks.size, r.store.size], [null, 0, 0, 0]);
}
{
  const r = rig(), o = go(r, idea);
  await settle();
  H.answer(r.env, H.FLOAT, H.take(r.env, H.FLOAT), { ok: false, why: "It has “B” open with text that is not saved." });
  await settle();
  eq("a refusal is its words", o.out, "It has “B” open with text that is not saved.");
}
{
  const r = rig(), o = go(r, idea);
  await settle();
  H.answer(r.env, H.FLOAT, H.take(r.env, H.FLOAT), { ok: true, doc: { kind: "idea", id: B } });
  await settle();
  eq("another document open there is not this one's answer", o.out, "The Ideas window opened another document instead.");
}
{
  const r = rig(), o = go(r, idea);
  await settle();
  H.answer(r.env, H.FLOAT, { ask: "ask-99" }, { ok: true, doc: idea });
  await r.tick(200);
  eq("an answer to another ask is not taken", o.out, undefined);
  await r.tick(H.ANSWER_MS);
  eq("nobody answers: said, and the ask is withdrawn", [o.out, r.store.has(H.FLOAT.ask), r.ears.length, r.ticks.size], ["The Ideas window did not open it.", false, 0, 0]);
}
{
  const r = rig(), o = go(r, idea, async () => { throw "no window"; });
  await settle();
  eq("a window that does not open is said at once, and the ask is withdrawn", [o.out, r.store.has(H.FLOAT.ask)], ["The Ideas window did not open: no window", false]);
}
{
  const r = rig();
  r.breaks();
  let raised = 0;
  const o = go(r, idea, async () => { raised++; });
  await settle();
  eq("no storage: said, and no window is opened for nothing", [o.out, raised], ["The Ideas window could not be asked: this machine's storage is not available.", 0]);
}
{
  const r = rig(), one = go(r, idea), two = go(r, mem);
  await settle();
  const a = H.take(r.env, H.FLOAT);
  H.answer(r.env, H.FLOAT, a, { ok: true, doc: mem });
  await settle();
  eq("a second ask replaces the first, which nobody answers", [two.out, one.out], [null, undefined]);
  await r.tick(H.ANSWER_MS);
  eq("…and the first says so", one.out, "The Ideas window did not open it.");
}
const at = 1_000_000, raw = (o) => JSON.stringify({ ask: "x", at, doc: idea, ...o });
eq("a stale ask, one from the future, and what is not one", [raw({ at: at - H.ASK_TTL_MS - 1 }), raw({ at: at + 60_000 }), raw({ ask: "" }), raw({ doc: { kind: "idea", id: "../x" } }), raw({ doc: { kind: "file", path: "/etc/passwd" } }), "{", "[]", null].map((x) => H.readAsk(x, at)), Array(8).fill(null));
eq("a memory path is the folder's own", ["../x.md", "a/../b.md", "/etc/x", "a//b", ".git/config", "a\\b", "", "a\u0000b"].map((path) => H.readAsk(raw({ doc: { ...mem, path } }), at)), Array(8).fill(null));
eq("…and a plain one is taken", H.readAsk(raw({ doc: mem }), at)?.doc, mem);
eq("Float and Dock are two ways", new Set([H.FLOAT.ask, H.FLOAT.answer, H.DOCK.ask, H.DOCK.answer]).size, 4);

console.log(failed ? `\ndoc-handoff-check: ${failed} failed` : "\ndoc-handoff-check: all ok");
process.exit(failed ? 1 : 0);
