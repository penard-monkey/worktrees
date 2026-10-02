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
// A third, from review: a doctor count under a button must be what THAT button
// clears. `shadowed` / `copy-stale` survive a plain relink by design, `no-slot`
// needs provision, `unknown-key` needs an edit — so the doctor side is split by
// remedy (`remedies()` in ProjectSheet.tsx, evaluated here from the real
// source too) and each row carries only its own share.
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

// `remedies` lives beside `issueCount` in ProjectSheet.tsx, which imports React
// and Tauri — so slice the ONE function out rather than import the file. It is
// self-contained (no helpers), which is what makes the slice honest.
const sheetSrc = read("../src/ProjectSheet.tsx");
const remAt = sheetSrc.indexOf("export function remedies(");
const remEnd = sheetSrc.indexOf("\n}\n", remAt) + 3;
if (remAt < 0) { console.error("FAIL ProjectSheet.tsx has no `export function remedies(`"); process.exit(1); }
const remJs = (await transformWithEsbuild(sheetSrc.slice(remAt, remEnd), "rem.ts", { loader: "ts", format: "esm" })).code;
const { remedies } = await import(`data:text/javascript;base64,${Buffer.from(remJs).toString("base64")}`);
const finding = (code, severity = "error") => ({ severity, code, place: "p", path: "x", message: code });
const report = (...codes) => ({ code: 2, schema_version: 1, error: null, findings: codes.map((c) => finding(c)) });
const H = (...codes) => { const r = report(...codes); return { issues: r.findings.length, error: null, remedies: remedies(r) }; };

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
  const pending = agent({ ...fixable.repo, pending: "agent-instructions", pending_on_origin: true });
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
  if (!/waiting to merge/.test(rows.find((r) => r.id === "agent-pending")?.label ?? "")) fail("a pushed fix must read as a PR waiting to merge");
  else ok("pending on origin → 'waiting to merge'");
  // A branch that never left the machine has NO PR — never say one is waiting.
  const local = projectTodos(null, agent({ ...fixable.repo, pending: "agent-instructions", pending_on_origin: false }));
  const lrow = local.find((r) => r.id === "agent-pending");
  if (!lrow || /waiting to merge/.test(lrow.label) || !/never pushed/.test(lrow.label)) {
    fail(`local-only pending branch reads "${lrow?.label}" — it must say it was never pushed, not that a PR waits`);
  } else ok("pending locally only → 'never pushed'");
  if (todoCount(local) !== 0) fail("a local-only pending branch must still count 0 (Fix would refuse: the branch exists)");
  else ok("a local-only pending branch counts 0");
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

// ── 4. doctor, split by what clears it ─────────────────────────────────────
{
  // THE review fixture: a shadowed file must not sit under a relink action.
  const shadow = projectTodos(H("shadowed", "not-linked"), null);
  const under = (id, action) => shadow.some((r) => r.id === id && r.action === action);
  const relinkN = shadow.filter((r) => r.action === "relink").reduce((a, r) => a + r.n, 0);
  if (relinkN !== 1) fail(`relink rows count ${relinkN} with one not-linked + one shadowed — a plain relink leaves shadowed alone`);
  else ok("relink counts only what relink clears (shadowed excluded)");
  if (!under("doctor-force", "force")) fail("shadowed has no re-seed (force) row");
  else ok("shadowed sits under the armed re-seed, not relink");

  const all = H("not-linked", "wrong-mode", "copy-stale", "no-slot", "missing-port", "unknown-key", "undeclared", "slot-conflict", "dangling-link");
  const rows = projectTodos(all, null);
  const n = (id) => rows.find((r) => r.id === id)?.n ?? -1;
  if (n("doctor-relink") !== 2) fail(`relink n=${n("doctor-relink")}, expected 2 (not-linked, wrong-mode)`);
  else ok("relink: not-linked + wrong-mode");
  if (n("doctor-force") !== 1) fail(`force n=${n("doctor-force")}, expected 1 (copy-stale)`);
  else ok("force: copy-stale");
  if (n("doctor-provision") !== 2) fail(`provision n=${n("doctor-provision")}, expected 2 (no-slot, missing-port)`);
  else ok("provision: no-slot + missing-port");
  const manual = rows.find((r) => r.id === "doctor-manual");
  if (!manual || manual.n !== 0 || manual.action !== null || !/4 /.test(manual.label)) {
    fail(`edit-only findings: ${JSON.stringify(manual)} — expected a 0-count row naming 4, with no button`);
  } else ok("edit-only findings (unknown-key, undeclared, slot-conflict, dangling-link) are a row with no count");
  const rs = all.remedies;
  if (rs.relink + rs.force + rs.provision + rs.stray + rs.registry + rs.manual !== all.issues) fail("remedies must sum to issueCount's population");
  else ok("the remedy split sums to the issue count");
  if (remedies(report("totally-new-code")).manual !== 1) fail("an unknown code must land in manual — the bucket that promises no button");
  else ok("an unknown code promises no button");
  // Registry notes are not .worktrees.toml's to fix — their own row, never manual.
  {
    const rr = remedies(report("nested-project", "prefix-collision"));
    if (rr.registry !== 2 || rr.manual !== 0) fail(`nested-project/prefix-collision: ${JSON.stringify(rr)} — expected registry 2, manual 0`);
    else ok("nested-project + prefix-collision land in their own bucket, not manual");
    const rrows = projectTodos({ issues: 2, error: null, remedies: rr }, null);
    const reg = rrows.find((r) => r.id === "doctor-registry");
    if (!reg || reg.n !== 0 || reg.action !== null || /\.worktrees\.toml/.test(reg.label) || rrows.some((r) => r.id === "doctor-manual")) {
      fail(`registry row: ${JSON.stringify(rrows)} — expected a 0-count info row that does not send you to .worktrees.toml`);
    } else ok("registry notes are a row that never says to edit .worktrees.toml");
  }
  const info = { code: 0, schema_version: 1, error: null, findings: [finding("port-busy", "info"), finding("copy-stale", "info")] };
  if (Object.values(remedies(info)).some((v) => v !== 0)) fail("info findings must not count (issueCount excludes them)");
  else ok("info findings count nowhere");

  if (count(H("not-linked", "no-slot", "shadowed"), fixable) !== 7) fail("doctor + agent must add up (3 + 4)");
  else ok("the project count is doctor's clearable findings + fixable agent items");
  const broken = projectTodos({ issues: 5, error: "bad toml", remedies: { relink: 5, force: 0, provision: 0, manual: 0 } }, null);
  if (todoCount(broken) !== 0) fail("a doctor that could not run must not show its stale issue count as a number");
  else ok("an unreadable config counts 0 (the last issues are not a fact)");
  if (!needsRepair(broken)) fail("an unreadable config must still offer Repair / upgrade");
  else ok("an unreadable config still offers Repair");
  if (projectTodos(H(), agent({ dirs: [{ dir: "", kind: "stub" }] })).length !== 0) {
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
  if (!/projectTodos\(/.test(sheetSrc)) fail("ProjectSheet's To do list does not use projectTodos");
  else ok("the sheet's To do list uses projectTodos");
  // The badge is a project fact: it must not wait for some place to be open
  // (CLAUDE.md: `sel` is a selection, `selected` a lookup that can be null).
  const at = app.indexOf("todo-mini|");
  const block = at < 0 ? "" : app.slice(app.lastIndexOf("{pv.ok && (() => {", at), app.indexOf("})()}", at));
  if (!block) fail("App.tsx: no todo-mini badge block found");
  else if (/\bsel\b|\bselected\b/.test(block)) fail("the todo-mini badge is gated on sel/selected — a project badge must not need an open place");
  else ok("the header badge is not gated on sel/selected");
  const agentSrc = read("../src/AgentSetup.tsx");
  if (!/const todo = todoCount\(projectTodos\(/.test(agentSrc)) fail("AgentSetupSection's 'N to fix' is recounted instead of derived from projectTodos");
  else ok("the section's 'N to fix' tag is derived from projectTodos");
}

console.log(bad ? `\n${bad} failure(s)` : "\nall good");
process.exit(bad ? 1 : 0);
