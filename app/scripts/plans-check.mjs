// Plans and their workspaces, the non-drag paths (app/src/plans.ts, idea
// board phase 4, §6.1): a check cannot drive a real drag (navdrag.ts's
// header), so it holds the rules every path goes through, the calls they end
// in, and what the windows SAY for core's reader.
//
//   node app/scripts/plans-check.mjs     # from the repo root; exits non-zero on failure
//
// It loads the REAL module (as a `data:` module, which is why plans.ts may
// have no runtime import) and holds:
//   - the drop rules of §6.1's four phase 4 rows: an idea onto a plan, a plan
//     onto a workspace with no plan (one plan, at most one workspace), an idea
//     onto a workspace, a project onto a plan; each null case is no target;
//   - every call names a console row serve has (dispatch.rs), with arguments
//     its body reads — so a drag, its menu path and a later agent's proposal
//     are one call (§7.1);
//   - the reader's words: the golden fixture core's reader is held to
//     (crates/worktrees-core/tests/fixtures/plans, a plan with a typo in
//     `## Work`) is said as "Line N: …", and N is the document's own line;
//   - in the windows, each drop and its menu path end in the same function,
//     and no plan or workspace row is invoked except through plans.ts;
//   - the mock answers every phase 4 row (it must track every command);
//   - the cross-window hand-off keys: a request is taken once, and a stale
//     or malformed one is ignored.
//
// Override, so a mutation can be shown red without touching the tree:
//   PLANS_SRC=<copy of plans.ts>  COMPANION_SRC=<copy of Companion.tsx>  APP_SRC=<copy of App.tsx>
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = process.env.PLANS_SRC ?? here("../src/plans.ts");
const COMP = process.env.COMPANION_SRC ?? here("../companion/Companion.tsx");
const APP = process.env.APP_SRC ?? here("../src/App.tsx");
let failed = 0;
const check = (c, m) => { if (!c) failed++; console.log(`${c ? "ok" : "not ok"} — ${m}`); };
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const src = fs.readFileSync(SRC, "utf8");
check(!/^\s*import\s(?!type\b)/m.test(src), "plans.ts has no runtime import (this check loads it as a data: module)");
const js = (await transformWithEsbuild(src, "plans.ts", { loader: "ts" })).code;
const P = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);

const A = "01JC8Z4K3M9QX2T7VBN5R6WYHA", B = "01JC8Z4K3M9QX2T7VBN5R6WYHB", PL = "01JC90A1000000000000000000", PL2 = "01JC90A1000000000000000002";
const W1 = "01MOCK00000000000000000001", W2 = "01MOCK00000000000000000002";

// ── the drop rules (§6.1) ───────────────────────────────────────────────────
const idea = { id: A, kind: "idea" }, plan = { id: PL, kind: "plan" };
check(P.ideaOntoPlan(idea, plan) === PL, "an idea onto a plan adds it to plan_of");
check(P.ideaOntoPlan(idea, { id: B, kind: "idea" }) === null, "an idea onto an idea is not a plan drop (that is phase 3's chooser)");
check(P.ideaOntoPlan(plan, { id: PL2, kind: "plan" }) === null, "a plan onto a plan is no target");
check(P.ideaOntoPlan(idea, plan, [A]) === null, "an idea the plan already names is no target");
check(P.ideaOntoPlan(idea, { ...plan, problem: "duplicate-id" }) === null && P.ideaOntoPlan(idea, null) === null, "a row with a problem, or nothing, is no target");
const ws = [{ id: W1, plan: null, ideas: [] }, { id: W2, plan: PL2, ideas: [A] }];
check(P.planOntoWorkspace(PL, ws[0], ws) === W1, "a plan onto a workspace with no plan sets it");
check(P.planOntoWorkspace(PL, ws[1], ws) === null, "a workspace that has a plan takes no other");
check(P.planOntoWorkspace(PL2, ws[0], ws) === null, "a plan that already has a workspace is no target anywhere else (one plan, at most one workspace)");
check(P.ideaOntoWorkspace(B, ws[1]) === W2 && P.ideaOntoWorkspace(A, ws[1]) === null && P.ideaOntoWorkspace(A, null) === null,
  "an idea onto a workspace links it, unless it is linked there already");
const view = { project_states: [{ line: 17, name: "worktrees", state: "registered", root: "/w/worktrees" }] };
check(P.projectOntoPlan("/w/other", view) && !P.projectOntoPlan("/w/worktrees", view) && !P.projectOntoPlan("rel", view) && !P.projectOntoPlan("/w/x", null),
  "a project onto a plan adds it unless the plan already lists that root");

// ── "Make a plan": which ideas ──────────────────────────────────────────────
const rows = [{ id: A, kind: "idea" }, { id: B, kind: "idea" }, { id: PL, kind: "plan" }, { id: "x", problem: "no-id" }];
check(same(P.planSources({ id: A, kind: "idea" }, [], rows), [A]), "nothing selected: the open idea");
check(same(P.planSources({ id: A, kind: "idea" }, [A, B, PL], rows), [A, B]), "the open idea among the selected: all the selected ideas, never a plan");
check(same(P.planSources({ id: A, kind: "idea" }, [B], rows), [A]), "a selection the open idea is not in: the open idea alone");
check(same(P.planSources(null, [B, A], rows), [B, A]) && same(P.planSources({ id: PL, kind: "plan" }, [], rows), []), "from the list: the selection; a plan alone makes no plan");

// ── each call is a console row, with arguments its body reads ───────────────
const dispatch = fs.readFileSync(here("../../crates/worktrees-cli/src/serve/dispatch.rs"), "utf8");
const camel = (k) => k.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
const rowLine = (cmd) => dispatch.split("\n").find((l) => l.trimStart().startsWith(`${cmd}: `)) ?? null;
const rowArgs = (cmd) => { const l = rowLine(cmd); return l ? [...l.matchAll(/a\.get\("([a-z_]+)"\)/g)].map((m) => camel(m[1])) : null; };
const calls = {
  make: P.makePlanCall([A, B, A], "  Home  "),
  makeBare: P.makePlanCall([A]),
  add: P.addIdeasCall(PL, [A, A], 5),
  remove: P.removeIdeaCall(PL, A, 6),
  project: P.addProjectCall(PL, "/w/worktrees", 7),
  fromPlan: P.fromPlanCall(PL),
  setPlan: P.setPlanCall(W1, PL),
  clearPlan: P.setPlanCall(W1, null),
  link: P.linkIdeaCall(W1, A),
  unlink: P.unlinkIdeaCall(W1, A),
};
for (const [name, c] of Object.entries(calls)) {
  const want = rowArgs(c.cmd);
  const extra = want ? Object.keys(c.args).filter((k) => !want.includes(k)) : null;
  check(want && extra.length === 0, `${name}: ${c.cmd} is a row, and reads every argument the page sends (${want ? (extra.length ? `unread: ${extra}` : Object.keys(c.args).join(", ")) : "no such row"})`);
  check(/HostConsole/.test(rowLine(c.cmd) ?? ""), `${c.cmd} is a console row (desktop only)`);
}
for (const cmd of ["plan_read", "workspace_overview"]) check(/HostConsole/.test(rowLine(cmd) ?? ""), `${cmd} is a console row`);
check(same(calls.make.args, { ideas: [A, B], title: "Home" }) && same(calls.makeBare.args, { ideas: [A] }),
  "make_plan(ideas, title?): each idea once, the title trimmed; no draft from the window (the template is core's)");
check(same(calls.add.args, { id: PL, ideas: [A], expectedMtime: 5 }) && calls.remove.args.idea === A && calls.project.args.root === "/w/worktrees",
  "the plan edits carry the plan's mtime, so an edit made elsewhere is refused rather than lost");
check(calls.clearPlan.args.plan === null, "a workspace's plan is cleared with null");

// ── the reader's words, on core's golden fixture ────────────────────────────
const FIX = here("../../crates/worktrees-core/tests/fixtures/plans/jellyfin-typo");
const md = fs.readFileSync(`${FIX}.md`, "utf8").split("\n");
const read = JSON.parse(fs.readFileSync(`${FIX}.json`, "utf8"));
const typo = read.problems.find((p) => p.text.startsWith("projet:"));
check(!!typo, "core's reader names the typo line in `## Work` (the golden fixture)");
if (typo) {
  check(md[typo.line - 1] === typo.text, `the problem's line number is the document's own line (${typo.line}: ${JSON.stringify(md[typo.line - 1])})`);
  const said = P.problemLine(typo);
  check(said.startsWith(`Line ${typo.line}: `) && said.includes("unknown key `projet:`"), `the windows say where and what: ${JSON.stringify(said)}`);
}
check(read.work.length === 2 && read.projects.length === 3, "the rest of the plan still reads around the bad line (both items, three projects)");
check(P.STATE_WORDS["to-clone"] === "to clone" && P.STATE_WORDS["to-create"] === "to create" && P.STATE_WORDS["not-registered"],
  "projects are listed as registered, to clone or to create");
check(P.workStatus({ started: null }) === "not started" && P.workStatus({ started: "a · b · 2026-11-12" }).includes("2026-11-12"), "a work item's started: line is shown as written");
check(same(P.plansIn([{ id: PL, how: "planned-in" }, { id: PL, how: "planned-in" }, { id: B, how: "related" }]).map((r) => r.id), [PL]),
  "an idea's plans are its planned-in relations, once each");

// ── the windows: one function per act, no second door ──────────────────────
const comp = fs.readFileSync(COMP, "utf8");
const app = fs.readFileSync(APP, "utf8");
const ROWS = ["plan_make", "plan_add_ideas", "plan_remove_idea", "plan_add_project", "workspace_from_plan", "workspace_set_plan", "workspace_link_idea", "workspace_unlink_idea"];
for (const cmd of ROWS) {
  check(!comp.includes(`"${cmd}"`) && !app.includes(`"${cmd}"`), `neither window invokes ${cmd} except through plans.ts's builder`);
}
check(/if \(target\.kind === "plan"\) \{ void doAddIdeas\(target\.id, target\.ids\)/.test(comp) && /void doAddIdeas\(p, \[r\.id!\]\)/.test(comp),
  "an idea dropped on a plan and the plan strip's Add idea… end in doAddIdeas");
check(/if \(target\.kind === "plan-projects"\) \{ void doAddProject\(target\.id, target\.root\)/.test(comp) && /void doAddProject\(planView\.doc\.id, root\)/.test(comp),
  "a project dropped on the plan's Projects and the Add project… click end in doAddProject");
check((comp.match(/void doMakeWorkspace\(/g) ?? []).length >= 2, "Make workspace: the plan strip and the list row's menu, one function");
check(/doMakePlan\(planSources\(open, selected, ideaRowsAll\)\)/.test(comp) && /doMakePlan\(planSources\(null, selected, ideaRowsAll\)\)/.test(comp),
  "Make a plan: from the open idea's strip and from the selection, one function, one rule");
check(/if \(target\.kind === "ws-plan"\) \{ void wsCall\(setPlanCall\(target\.id, target\.plan\)\)/.test(app) && /setPlan=\{\(plan\) => void wsCall\(setPlanCall\(wsOpen, plan\)\)\}/.test(app),
  "a plan dropped on a workspace and the overview's Link a plan ▸ end in the same call");
check(/if \(target\.kind === "ws-idea"\) \{ void wsCall\(linkIdeaCall\(target\.id, target\.idea\)\)/.test(app) && /linkIdea=\{\(id\) => void wsCall\(linkIdeaCall\(wsOpen, id\)\)\}/.test(app),
  "an idea dropped on a workspace and the overview's Link an idea ▸ end in the same call");
check(/planOntoWorkspace\(item\.id, onto, all\)/.test(app) && /ideaOntoWorkspace\(item\.id, onto\)/.test(app), "the main window's drop resolves through plans.ts's rules");
check(/ideaOntoPlan\(/.test(comp) && /projectOntoPlan\(item\.root, planView\)/.test(comp), "the companion's drop resolves through plans.ts's rules");
check(/<PlanPanel[\s\S]{0,200}view=\{planView\}/.test(comp) && /\{v\.problems\.length > 0 &&/.test(fs.readFileSync(here("../companion/PlanPanel.tsx"), "utf8")),
  "a plan's problems are drawn above the editor, which shows the document whatever they are");

// ── the mock answers every phase 4 row ──────────────────────────────────────
const mock = fs.readFileSync(here("../src/mock/install.ts"), "utf8");
for (const cmd of [...ROWS, "plan_read", "workspace_overview"]) check(mock.includes(`case "${cmd}":`), `the mock answers ${cmd}`);
const ipc = fs.readFileSync(here("../src/ipc.ts"), "utf8");
const consoleRows = ipc.slice(ipc.indexOf("export const CONSOLE_ROWS"), ipc.indexOf("];", ipc.indexOf("export const CONSOLE_ROWS")));
for (const cmd of [...ROWS, "plan_read", "workspace_overview"]) check(consoleRows.includes(`"${cmd}"`), `${cmd} is in ipc.ts's CONSOLE_ROWS`);

// ── the hand-off between the windows ────────────────────────────────────────
const now = 1_800_000_000_000;
check(P.showRequest(JSON.stringify({ id: A, at: now - 1000 }), now) === A, "a fresh request names its document");
check(P.showRequest(JSON.stringify({ id: A, at: now - P.SHOW_TTL_MS - 1 }), now) === null, "a request older than a minute is ignored");
check(P.showRequest(JSON.stringify({ id: "../x", at: now }), now) === null && P.showRequest("{", now) === null && P.showRequest(null, now) === null,
  "a malformed request is ignored");
check(/window\.localStorage\.removeItem\(SHOW_KEY\)/.test(comp) && /window\.localStorage\.removeItem\(WS_SHOW_KEY\)/.test(app),
  "each window takes a request once (removes the key it read)");

console.log(failed ? `\nplans-check: ${failed} failure(s)` : "\nplans-check: all ok");
process.exit(failed ? 1 : 0);
