// Checks the frontend's harness list — app/src/harness.ts — against the one it
// mirrors, core's registry (`provider::PROVIDERS` in provider.rs), and the
// model picker's preselect rule (app/src/ModelPicker.tsx `defaultModel`).
//
//   node app/scripts/harness-check.mjs        # exits non-zero on failure
//
// Why a mirror needs this: a row added to the registry and not here is a
// harness the app silently never offers (every list filters by HARNESSES), and
// one added here and not there is a picker option the backend refuses. Neither
// fails a test that only reads one side.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) { console.log(`ok ${name}`); return; }
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};

// ── registry order ──────────────────────────────────────────────────────────
const rs = read("../../crates/worktrees-core/src/provider.rs").split("#[cfg(test)]")[0];
const table = rs.slice(rs.indexOf("pub const PROVIDERS"), rs.indexOf("];", rs.indexOf("pub const PROVIDERS")));
const coreIds = [...table.matchAll(/^\s*id: "([^"]+)",/gm)].map((m) => m[1]);
const needsModel = [...table.matchAll(/model_arg: (Some\("[^"]+"\)|None)/g)].map((m) => m[1] !== "None");

const hjs = (await transformWithEsbuild(read("../src/harness.ts"), "harness.ts", { loader: "ts", format: "esm" })).code;
const H = await import("data:text/javascript;base64," + Buffer.from(hjs).toString("base64"));
eq("HARNESSES is the registry, in registry order", [...H.HARNESSES], coreIds);
eq("every harness has a label", H.HARNESSES.map((h) => typeof H.HARNESS_LABEL[h]), H.HARNESSES.map(() => "string"));
eq("every harness is in NEEDS_MODEL", H.HARNESSES.map((h) => typeof H.NEEDS_MODEL[h]), H.HARNESSES.map(() => "boolean"));
eq("a harness that needs a model takes one (model_arg)", H.HARNESSES.map((h, i) => !H.NEEDS_MODEL[h] || needsModel[i]), H.HARNESSES.map(() => true));
eq("pi needs a model; claude and codex do not", [H.NEEDS_MODEL.pi, H.NEEDS_MODEL.claude, H.NEEDS_MODEL.codex], [true, false, false]);

// ── the preselect rule, from the real source ────────────────────────────────
const mp = read("../src/ModelPicker.tsx");
const slice = (from, to) => mp.slice(mp.indexOf(from), mp.indexOf(to, mp.indexOf(from)));
const src = [
  slice("export const modelArg", "\n\n") ,
  slice("export function defaultModel", "\nconst OTHER"),
].join("\n").replace(/export /g, "");
const js = (await transformWithEsbuild(`const NEEDS_MODEL = ${JSON.stringify(H.NEEDS_MODEL)};\n${src}\nexport { defaultModel };`,
  "pick.ts", { loader: "ts", format: "esm" })).code;
const { defaultModel } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
const opt = (backend, model, ready) => ({ model: { harness: "pi", backend, model, label: null }, ready, reason: ready ? null : "no_credentials", source: "t", meta: {} });
const pi = [opt("kimi-coding", "k3", false), opt("lm-studio", "qwen3.6-27b", true), opt("lm-studio", "other", true)];
eq("pi with no default preselects its first READY model", defaultModel(pi, undefined, "pi"), "lm-studio/qwen3.6-27b");
eq("pi's configured default wins when ready", defaultModel(pi, "lm-studio/other", "pi"), "lm-studio/other");
eq("a configured default that is not ready falls to the first ready", defaultModel(pi, "kimi-coding/k3", "pi"), "lm-studio/qwen3.6-27b");
eq("a free-text default the catalog does not list is kept", defaultModel(pi, "x/y", "pi"), "x/y");
eq("pi with nothing ready preselects nothing", defaultModel([pi[0]], undefined, "pi"), "");
const aliases = [{ ...opt(null, "opus", true), model: { harness: "claude", backend: null, model: "opus", label: null } }];
eq("claude with no default keeps the CLI's own (never the first alias)", defaultModel(aliases, undefined, "claude"), "");
eq("claude's configured default is used", defaultModel(aliases, "opus", "claude"), "opus");

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
