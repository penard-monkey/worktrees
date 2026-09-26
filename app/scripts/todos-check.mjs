// Guards the per-project to-do model — what a project's number COUNTS.
//
//   node app/scripts/todos-check.mjs        # exits non-zero on failure
//
// WHY THIS EXISTS. The number on a project's header, the "Repair / upgrade…"
// entry and the sheet's To do list all come from `projectTodos`, so one wrong
// rule is wrong on three surfaces at once — and nothing else would notice: the
// mock answers instantly and correctly, and every value "looks" plausible. The
// two rules that are easiest to get wrong are the ones where the backend says
// `fixable: true` and the right answer is still ZERO:
//
//   1. A PENDING fix PR counts nothing. The default branch only moves when the
//      PR merges, so the report stays `fixable` the whole time — a count built
//      on `fixable` alone badges a project whose only step left is a review.
//   2. DIVERGED dirs count nothing. No button can merge two instruction files,
//      and a number no button can lower is a nag.
//
// Plus: the user's own skills are machine-wide and never a per-project to-do,
// and a doctor that could not run is a row (and a reason to offer Repair), but
// never a number.
//
// Same slice-the-real-source shape as offers-check.mjs: it evaluates
// `projectTodos.ts` itself, so it tests the edit and not a paraphrase.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// vite's esbuild re-export — see race-check.mjs: bare "esbuild" does not resolve
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

const src = read("../src/projectTodos.ts").replace(/^import type .*$/gm, "");
const js = (await transformWithEsbuild(src, "todos-check.ts", { loader: "ts", format: "esm" })).code;
const { projectTodos, todoCount, needsRepair } = await import(
  `data:text/javascript;base64,${Buffer.from(js).toString("base64")}`
);

const agent = (repo, user = []) => ({
  repo: { reference: "origin/main", dirs: [], skills: [], fixable: false, conflicts: false, pending: null, ...repo },
  user_skills: user,
});
const fixable = agent({
  dirs: [{ dir: "", kind: "claude-only" }, { dir: "a", kind: "claude-only" }, { dir: "b", kind: "agents-only" }, { dir: "c", kind: "stub" }],
  skills: [{ name: "deploy", kind: "missing" }, { name: "review", kind: "present" }],
  fixable: true,
});
const count = (h, a) => todoCount(projectTodos(h, a));

// ── the fixable case counts what the Fix PR carries ────────────────────────
{
  const n = count(null, fixable);
  if (n !== 4) fail(`fixable: count ${n}, expected 4 (2 claude-only + 1 agents-only + 1 missing skill; stub/present are done)`);
  else ok("fixable agent setup counts dirs + missing repo skills");
  const t = projectTodos(null, fixable);
  if (!t.every((x) => x.id.startsWith("agent-") ? x.action === "agent-fix" : true)) fail("fixable rows must carry the agent-fix action");
  else ok("fixable rows act through the one Fix PR");
}

// ── 1. pending → 0 ─────────────────────────────────────────────────────────
{
  const pending = agent({ ...fixable.repo, pending: "agent-instructions" });
  const n = count(null, pending);
  if (n !== 0) fail(`pending PR: count ${n}, expected 0 — the base ref stays fixable until the merge`);
  else ok("a pending fix PR counts 0");
  const rows = projectTodos(null, pending);
  if (!rows.some((r) => r.id === "agent-pending")) fail("pending PR: no informational row — the sheet would say nothing about it");
  else ok("a pending fix PR still gets a row");
  if (rows.some((r) => r.action === "agent-fix")) fail("pending PR: a row still offers the Fix — that is a second push");
  else ok("a pending fix PR offers no second Fix");
  if (needsRepair(rows)) fail("pending PR alone must not put Repair / upgrade in the menu");
  else ok("pending alone does not offer Repair");
}

// ── 2. diverged is not counted ─────────────────────────────────────────────
{
  const div = agent({ dirs: [{ dir: "x", kind: "diverged" }, { dir: "y", kind: "diverged" }], conflicts: true, fixable: false });
  const n = count(null, div);
  if (n !== 0) fail(`diverged only: count ${n}, expected 0 — merging by hand is not a button`);
  else ok("diverged dirs count 0");
  if (!projectTodos(null, div).some((r) => r.id === "agent-diverged" && r.action === null)) fail("diverged: no merge-by-hand row");
  else ok("diverged dirs get a merge-by-hand row with no button");
  // …and alongside fixable ones they add nothing either
  const mixed = agent({ ...fixable.repo, dirs: [...fixable.repo.dirs, { dir: "z", kind: "diverged" }], conflicts: true });
  if (count(null, mixed) !== 4) fail(`fixable + diverged: count ${count(null, mixed)}, expected 4`);
  else ok("diverged dirs add nothing to a fixable count");
}

// ── 3. user skills are machine-wide, never a project to-do ─────────────────
{
  const user = agent({}, [{ name: "a", status: "missing" }, { name: "b", status: "missing" }]);
  if (projectTodos(null, user).length !== 0) fail("missing USER skills produced a project to-do — it would stand under every project");
  else ok("user skills are not a per-project to-do");
}

// ── 4. doctor ──────────────────────────────────────────────────────────────
{
  if (count({ issues: 3, error: null }, null) !== 3) fail("doctor issues must count");
  else ok("doctor issues count");
  if (count({ issues: 3, error: null }, fixable) !== 7) fail("doctor + agent must add up (3 + 4)");
  else ok("the project count is doctor + fixable agent items");
  const broken = projectTodos({ issues: 5, error: "bad toml" }, null);
  if (todoCount(broken) !== 0) fail("a doctor that could not run must not show its stale issue count as a number");
  else ok("an unreadable config counts 0 (the last issues are not a fact)");
  if (!needsRepair(broken)) fail("an unreadable config must still offer Repair / upgrade");
  else ok("an unreadable config still offers Repair");
  if (projectTodos({ issues: 0, error: null }, agent({ dirs: [{ dir: "", kind: "stub" }] })).length !== 0) {
    fail("a clean project produced a to-do");
  } else ok("a clean project has nothing to do");
  if (projectTodos(undefined, undefined).length !== 0) fail("unknown health/agent (not probed yet) must produce nothing");
  else ok("nothing probed yet → nothing to do");
}

// ── 5. the surfaces read this module, not their own arithmetic ─────────────
{
  const app = read("../src/App.tsx");
  if (!/projectTodos\(health\[/.test(app)) fail("App.tsx does not build the header/menu count from projectTodos(health[…], …)");
  else ok("App builds the count from projectTodos");
  if (/AgentSetupBanner|agent_setup_dismissed|agentSetupOffers/.test(app)) {
    fail("App.tsx still references the retired nav banner (AgentSetupBanner/agent_setup_dismissed/agentSetupOffers)");
  } else ok("the nav agent-setup banner is gone");
  const sheet = read("../src/ProjectSheet.tsx");
  if (!/projectTodos\(/.test(sheet)) fail("ProjectSheet's To do list does not use projectTodos");
  else ok("the sheet's To do list uses projectTodos");
}

console.log(bad ? `\n${bad} failure(s)` : "\nall good");
process.exit(bad ? 1 : 0);
