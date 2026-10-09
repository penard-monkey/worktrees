// Guards the Plan tab's side of a CLOSED union: `PlanSummary.how_resolved`.
//
//   node app/scripts/planresolve-check.mjs      # exits non-zero on failure
//
// `plan::Resolved` (core) is matched by name on this side. Owned planning grew
// it from three values to six (owned-planning §3.2: "new values are not
// additive"), and a value the frontend's union lacks still type-checks at the
// invoke boundary — it would render as whatever the fallthrough says. So this
// parses core's enum and requires:
//
//   1. PlanPane's union lists exactly core's variants (snake_case) plus null;
//   2. `resolutionLine` gives every variant its OWN words at show/full — none
//      may reach the default arm — and says nothing at all at off, which must
//      render exactly as before.
//
// Same slice-the-real-source shape as offers-check.mjs: it evaluates the real
// function, not a paraphrase.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

const rust = read("../../crates/worktrees-core/src/plan.rs");
const en = rust.match(/pub enum Resolved \{([\s\S]*?)\n\}/);
if (!en) { fail("plan.rs: no `pub enum Resolved`"); process.exit(1); }
const variants = [...en[1].matchAll(/^\s{4}([A-Z][A-Za-z]*),/gm)]
  .map((m) => m[1].replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase());
if (variants.length < 6) fail(`parsed only [${variants}] from plan::Resolved — the parser drifted`);

const pane = read("../src/PlanPane.tsx");
const u = pane.match(/how_resolved:\s*([^;]+);/);
const union = u ? [...u[1].matchAll(/"([a-z_]+)"/g)].map((m) => m[1]) : [];
const want = [...variants].sort().join(",");
if (union.slice().sort().join(",") !== want || !/\bnull\b/.test(u?.[1] ?? "")) {
  fail(`PlanPane's how_resolved union is [${union}] (+null?) — core's plan::Resolved is [${variants}]`);
} else ok(`PlanPane's union matches plan::Resolved: ${variants.join(", ")} + null`);

// Slice out `resolutionLine` and evaluate it.
const at = pane.indexOf("export function resolutionLine(");
if (at < 0) { fail("PlanPane.tsx has no resolutionLine"); process.exit(1); }
const end = pane.indexOf("\n}\n", at);
const fnSrc = pane.slice(at, end + 2).replace(/^export /, "")
  .replace(/\(p: Pick<[^>]*>\): string \| null/, "(p)");
const js = (await transformWithEsbuild(`${fnSrc}\nexport { resolutionLine };`, "x.ts", { loader: "ts", format: "esm" })).code;
const { resolutionLine } = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);

const base = { topic: "t", plan_rel: "task_plan.md", plan_scope: null, reason: null };
for (const how of [...variants, null]) {
  if (resolutionLine({ ...base, level: "off", how_resolved: how }) !== null) fail(`off must render nothing new (how_resolved=${how})`);
}
ok("off renders no resolution line, for every value");
const seen = new Map();
for (const level of ["full", "show"]) {
  for (const how of [...variants, null]) {
    const line = resolutionLine({ ...base, level, how_resolved: how });
    if (!line || line.startsWith("resolved: ")) fail(`${level}/${how}: fell through to the default arm (${JSON.stringify(line)})`);
    if (level === "full") seen.set(how, line);
  }
}
if (new Set(seen.values()).size !== seen.size) fail(`two values share one line: ${JSON.stringify([...seen])}`);
else ok("every value has its own words at full and show");
const inv = resolutionLine({ ...base, level: "full", how_resolved: "invalid_pointer", plan_rel: null });
if (/\bt\b|\.planning\/t/.test(inv.replace(".planning/.active_plan", ""))) fail(`invalid_pointer echoes a topic: ${inv}`);
else ok("invalid_pointer names no topic or path to write to");
const main = resolutionLine({ ...base, level: "show", how_resolved: "show_path", plan_rel: "docs/goals.md", plan_scope: "main" });
if (!/main's copy/.test(main)) fail(`main scope is not labelled as main's copy: ${main}`);
else ok("main scope is labelled main's copy, never this place's own plan");

console.log(bad ? `\n${bad} failure(s)` : "\nall good");
process.exit(bad ? 1 : 0);
