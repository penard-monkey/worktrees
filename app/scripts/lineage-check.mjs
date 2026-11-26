// The mock's copy of core's lineage rule must give core's answers.
//
// Lane children (docs/proposals/lane-children.md): core resolves
// `Place.lineage` (`worktrees_core::lineage`) and the app only draws it. The
// mock harness stands in for core, so `app/src/mock/install.ts` carries a
// second implementation, `lineageOf` — a frontend that mirrors a core DECISION
// (AGENTS.md), which lies quietly unless something holds it to the original.
//
// This is that hold. The cases live in ONE file,
// `crates/worktrees-core/tests/fixtures/lineage/cases.json`, whose `expect` is
// core's answer: core's test `the_shared_cases_are_this_rules_answers` fails
// when its rule stops giving them. Here the REAL `lineageOf` source is sliced
// out of install.ts, compiled, fed the same places, and compared key by key.
// Run from the repo root, like every *-check.mjs.

import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };

const src = read("../src/mock/install.ts");
const START = 'const LINEAGE_MAIN = "(main)";';
const END = "/** Every snapshot's rows with `lineage`";
const a = src.indexOf(START), b = src.indexOf(END);
if (a < 0 || b < a) {
  console.error("FAIL could not find lineageOf in app/src/mock/install.ts (its markers moved)");
  process.exit(1);
}
if (src.indexOf(START, a + 1) >= 0) fail("install.ts has two LINEAGE_MAIN declarations: which one is the resolver?");
const body = "type Place = any; type Lineage = any;\n" + src.slice(a, b) + "\nexport { lineageOf };\n";
const js = (await transformWithEsbuild(body, "lineage.ts", { loader: "ts", format: "esm" })).code;
const { lineageOf } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

const file = JSON.parse(read("../../crates/worktrees-core/tests/fixtures/lineage/cases.json"));
if (!Array.isArray(file.cases) || file.cases.length < 10) fail(`expected the shared cases, found ${file.cases?.length}`);

// Key order is the wire's business, not the rule's: compare canonically.
const canon = (v) =>
  Array.isArray(v) ? v.map(canon)
  : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])]))
  : v;

for (const c of file.cases) {
  if (!c.expect || !Object.keys(c.expect).length) { fail(`${c.name}: no expect (bless it in core first)`); continue; }
  const live = new Set(c.live);
  // A mock Place carries what the resolver reads: slug, birth, declared, and
  // liveness as an agent session that is up.
  const places = c.places.map((p) => ({
    slug: p.slug,
    created_epoch: p.born,
    declared: c.declared[p.slug] ?? null,
    tmux_session: { name: p.slug, up: live.has(p.slug) },
    agent_sessions: {},
  }));
  const got = lineageOf(places);
  for (const [slug, want] of Object.entries(c.expect)) {
    const g = JSON.stringify(canon(got.get(slug) ?? {}));
    const w = JSON.stringify(canon(want));
    if (g !== w) fail(`${c.name} / ${slug}: mock ${g}, core ${w}`);
  }
}

if (bad) {
  console.error(`lineage-check: ${bad} failure(s)`);
  process.exit(1);
}
console.log(`lineage-check: the mock's resolver gives core's answer in all ${file.cases.length} shared cases`);
