// The pi model-hosts extension (crates/worktrees-core/src/pihosts_ext.js) is
// code in EVERY pi worktrees starts, so it must never take a launch down and
// must hand pi only what pi can list. This runs the REAL source under plain
// node with a stub `pi`, against data files written raw:
//
//   - a missing file: nothing registered, and SILENT (a fresh install has none);
//   - a malformed file: nothing registered, one line said, no throw;
//   - a host with no http(s) baseUrl, or no models: left out, said;
//   - a model with only an `id`: every field pi reads is filled — pi 1.5.0
//     applies none of models.json's defaults to registerProvider, and a model
//     without maxTokens crashes `pi --list-models`;
//   - a worktrees host gets its id as a placeholder key; an override gets no
//     key (pi then uses its own) — and the file's own `apiKey` is ignored;
//   - a pi without registerProvider (before 0.50.0), and a registerProvider
//     that throws: no throw.
//
//   node app/scripts/pi-models-ext-check.mjs   # exits non-zero on failure
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const SRC = fs.readFileSync(path.join(ROOT, "crates/worktrees-core/src/pihosts_ext.js"), "utf8");
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wt-pi-ext-"));
const data = path.join(dir, "pi-models.json");
const mod = path.join(dir, "ext.mjs");
fs.writeFileSync(mod, SRC.replace("__DATA__", JSON.stringify(data)));
const ext = (await import(pathToFileURL(mod).href)).default;

let failed = 0;
const fail = (m) => { failed++; console.log("FAIL " + m); };
const eq = (a, b, m) => { if (JSON.stringify(a) !== JSON.stringify(b)) fail(`${m}: ${JSON.stringify(a)} != ${JSON.stringify(b)}`); };

/** Run the extension once; what it registered and what it said. */
function run(file, pi) {
  if (file === null) fs.rmSync(data, { force: true }); else fs.writeFileSync(data, file);
  const said = [];
  const reg = [];
  const orig = console.error;
  console.error = (...a) => said.push(a.join(" "));
  try {
    ext(pi ?? { registerProvider: (id, p) => reg.push([id, p]) });
  } catch (e) {
    fail(`threw: ${e}`);
  } finally {
    console.error = orig;
  }
  return { reg, said };
}

let r = run(null);
eq(r.reg, [], "missing: nothing registered");
eq(r.said, [], "missing: silent");

r = run("{\"hosts\":");
eq(r.reg, [], "malformed: nothing registered");
if (r.said.length !== 1 || !r.said[0].includes("not valid JSON")) fail(`malformed: said ${JSON.stringify(r.said)}`);

r = run(JSON.stringify({ version: 1, hosts: {
  nourl: { api: "openai-completions", models: [{ id: "a" }] },
  fileurl: { baseUrl: "file:///etc", models: [{ id: "a" }] },
  nomodels: { baseUrl: "http://h:1/v1", models: [] },
  box: { baseUrl: "http://h:1/v1", apiKey: "sk-from-file", models: [{ id: "only-id" }, { name: "no id" }, { id: "full", name: "Full", contextWindow: 32768, maxTokens: 4096, reasoning: true, images: true }] },
  "lm-studio": { baseUrl: "http://h:2/v1", api: "openai-responses", override: true, models: [{ id: "q" }] },
} }));
eq(r.reg.map(([id]) => id), ["box", "lm-studio"], "only usable hosts are registered");
eq(r.said.length, 3, `three hosts left out, each said: ${JSON.stringify(r.said)}`);
const [, box] = r.reg[0] ?? [];
const [, lms] = r.reg[1] ?? [];
eq(box?.apiKey, "box", "a worktrees host's key is its id, never the file's");
eq(box?.api, "openai-completions", "api defaults");
eq(box?.models, [
  { id: "only-id", name: "only-id", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 128000, maxTokens: 16384 },
  { id: "full", name: "Full", reasoning: true, input: ["text", "image"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 32768, maxTokens: 4096 },
], "a model with only an id gets every field");
eq("apiKey" in (lms ?? {}), false, "an override carries no key");
eq(lms?.api, "openai-responses", "an override keeps its api");

r = run(JSON.stringify({ hosts: { box: { baseUrl: "http://h:1/v1", models: [{ id: "m" }] } } }), {});
eq(r.reg, [], "a pi without registerProvider: nothing, no throw");
r = run(JSON.stringify({ hosts: { a: { baseUrl: "http://h:1/v1", models: [{ id: "m" }] }, b: { baseUrl: "http://h:1/v1", models: [{ id: "m" }] } } }),
  { registerProvider: (id) => { if (id === "a") throw new Error("pi says no"); r.reg.push(id); } });
if (!r.said.some((s) => s.includes("pi says no"))) fail("a throwing registerProvider is said");

fs.rmSync(dir, { recursive: true, force: true });
if (failed) { console.log(`pi-models-ext-check: ${failed} failure(s)`); process.exit(1); }
console.log("pi-models-ext-check: all good");
