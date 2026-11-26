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
//     onto a workspace, a project onto a plan; each null case is no target,
//     and Everything Else (derived, never stored) is one for neither;
//   - every call names a console row serve has (dispatch.rs), with arguments
//     its body reads — so a drag, its menu path and a later agent's proposal
//     are one call (§7.1);
//   - the reader's words: the golden fixture core's reader is held to
//     (crates/worktrees-core/tests/fixtures/plans, a plan with a typo in
//     `## Work`) is said as "Line N: …", and N is the document's own line;
//   - in the windows, each drop and its menu path end in the same function,
//     and no plan or workspace row is invoked except through plans.ts;
//   - phase 5: where a work item stands (from the snapshot, never stored), what
//     may start it or take a place, its calls; the review in front of every
//     Start and one item per Start; Clone and Create through the main window's
//     own dialogs; a place dropped on an item and Link a place… in one call;
//   - the mock answers every phase 4 and 5 row (it must track every command);
//   - the cross-window hand-off keys: a request is taken once, and a stale
//     or malformed one is ignored.
//
// Override, so a mutation can be shown red without touching the tree:
//   PLANS_SRC=<copy of plans.ts>  COMPANION_SRC=<copy of Companion.tsx>  APP_SRC=<copy of App.tsx>
//   OVERVIEW_SRC=<copy of WorkspaceOverview.tsx>  PANEL_SRC=<copy of PlanPanel.tsx>
//   START_SRC=<copy of StartWork.tsx>
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = process.env.PLANS_SRC ?? here("../src/plans.ts");
const COMP = process.env.COMPANION_SRC ?? here("../companion/Companion.tsx");
const APP = process.env.APP_SRC ?? here("../src/App.tsx");
const OVERVIEW = process.env.OVERVIEW_SRC ?? here("../src/WorkspaceOverview.tsx");
const PANEL = process.env.PANEL_SRC ?? here("../companion/PlanPanel.tsx");
const START = process.env.START_SRC ?? here("../src/StartWork.tsx");
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
// Everything Else: the reserved group is not a workspace to link to (#684).
const { EVERYTHING_ELSE_ID: EE } = await import(`data:text/javascript;base64,${Buffer.from((await transformWithEsbuild(fs.readFileSync(here("../src/workspaces.ts"), "utf8"), "workspaces.ts", { loader: "ts" })).code).toString("base64")}`);
const ee = { id: EE, plan: null, ideas: [] };
check(typeof EE === "string" && EE.length > 0, "workspaces.ts names the reserved id");
check(P.planOntoWorkspace(PL, ee, [...ws, ee]) === null, "a plan onto Everything Else is no target");
check(P.ideaOntoWorkspace(B, ee) === null, "an idea onto Everything Else is no target");
check(P.showRequest(JSON.stringify({ id: EE, at: 1000 }), 1000) === null, "the Ideas window cannot ask to show Everything Else");
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
  setProject: P.setProjectCall(PL, "ans", "/w/ans", 8),
  linkPlace: P.linkPlaceCall(PL, { line: 23, title: "Role" }, "/w/ans", "role", 9),
  start: P.startCall(PL, { line: 23, title: "Role", branch: "feat/role" }, "/w/ans", "Do it.", { provider: "claude", model: "" }, 10),
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

// ── phase 5: a work item, from the snapshot ────────────────────────────────
const R = "/w/ans", none = new Set();
const pl = (slug, branch, o = {}) => ({ slug, path: `${R}/.worktrees/${slug}`, branch, is_main: false, tmux_session: { up: false }, ...o });
const snap = { projects: [{ root: R, name: "ans", ok: true, snapshot: { places: [pl("(main)", "main", { is_main: true }), pl("role", "feat/x"), pl("other", "o")] } }] };
const it = (o = {}) => ({ line: 23, title: "Role", project: "ans", branch: "feat/role", brief: "Do it.", ai: null, model: null, started: null, place: null, ...o });
const st = (o = {}) => ({ line: 23, project_state: "registered", root: R, ...o });
const started = (slug) => it({ started: `${R} · ${slug} · 2026-11-12 09:30`, place: { project: R, slug, at: "2026-11-12 09:30" } });
const key = `${R}|nope|2026-11-12 09:30`;
const at = (i, s, sn = snap, act = { busy: none, waiting: none }, gone = new Set([key])) => P.itemPlace(i, s, sn, act, gone);
const done = st({ place_root: R });
check(at(it(), st()).kind === "none" && P.workStatus(it(), at(it(), st())) === "not started", "an item with no started: line is not started");
check(at(started("role"), done).kind === "there" && P.workStatus(started("role"), at(started("role"), done)) === "started 2026-11-12 09:30 · there", "its place is there");
check(at(started("role"), done, snap, { busy: new Set([`${R}/.worktrees/role`]), waiting: none }).kind === "busy", "…busy, from the same activity the nav dots use");
check(at(started("role"), done, snap, { busy: none, waiting: new Set([`${R}/.worktrees/role`]) }).kind === "waiting", "…or waiting");
check(at(started("nope"), done).kind === "gone" && P.workStatus(started("nope"), at(started("nope"), done)).endsWith("· place removed"), "a removed place reads “place removed”, its line kept");
check(at(started("nope"), done, snap, { busy: none, waiting: none }, none).kind === "unknown", "missing from a list older than its line is not removed yet");
check(same(P.missingPlaces([started("nope"), started("role"), it()], [done, done, st()], snap), [key]) && same(P.missingPlaces([started("nope")], [done], null), []), "…it is what the window asks the list about again");
check(/useGone\(/.test(fs.readFileSync(APP, "utf8")) && /useGone\(/.test(fs.readFileSync(COMP, "utf8")), "…in both windows");
check(at(started("role"), done, null).kind === "unknown" && at(started("role"), done, { projects: [{ root: R, ok: false, snapshot: null }] }).kind === "unknown", "no snapshot, or one that failed, is never called removed");
check(at(started("role"), st()).kind === "unregistered" && at(it({ started: "yesterday" }), st()).kind === "unreadable", "an unregistered project, and a line that is not three parts, are said");
check(P.itemFree({ kind: "none" }) && P.itemFree({ kind: "gone" }) && P.itemFree({ kind: "unregistered" }) && !["there", "busy", "waiting", "unknown", "unreadable"].some((k) => P.itemFree({ kind: k })),
  "only an item with no place, or one whose place is gone, takes a Start or a link");
check(same(P.placeOntoItem(R, "other", it(), at(it(), st()), snap), { line: 23, title: "Role" }), "a place dropped on a free item is its place");
check(P.placeOntoItem(R, "other", started("role"), at(started("role"), done), snap) === null && P.placeOntoItem(R, "nope", it(), at(it(), st()), snap) === null && P.placeOntoItem(R, "other", null, null, snap) === null,
  "…and is no target on an item with a place, for a place that is not there, or on nothing");
check(P.startProblem(it(), st(), at(it(), st()), snap) === null, "a free item with a registered project and a free branch can start");
for (const [i, s, words] of [
  [it({ project: null }), st({ project_state: null, root: undefined }), "no project"],
  [it(), st({ project_state: "to-clone", root: undefined }), "not cloned yet"],
  [it(), st({ project_state: "to-create", root: undefined }), "not created yet"],
  [it(), st({ project_state: "not-registered", root: undefined }), "not a registered project"],
  [it({ branch: null }), st(), "no branch"],
  [it({ branch: "o" }), st(), "already in the place other"],
  [it({ branch: "main" }), st(), "main checkout"],
  [it({ branch: "other" }), st(), "a place named other already"],
  [started("role"), done, "has a place"],
]) check((P.startProblem(i, s, at(i, s), snap) ?? "").includes(words), `refused, said: ${words}`);
check(same(P.agentFor(it(), { provider: "claude", models: { claude: "opus" } }), { provider: "claude", model: "opus" }) && same(P.agentFor(it({ ai: "codex" }), { provider: "claude", models: { claude: "opus" } }), { provider: "codex", model: "" })
  && same(P.agentFor(it({ model: "sonnet" }), { provider: "claude", models: { claude: "opus" } }), { provider: "claude", model: "sonnet" }), "the agent is the app's default unless the item says otherwise; a default model never goes to another harness");
check(same(P.newPath("~/workspace/jelly/"), { location: "~/workspace", name: "jelly" }) && same(P.newPath("/x"), { location: "/", name: "x" }), "a `new:` path is split for the New project dialog");
check(same(calls.start.args, { id: PL, line: 23, title: "Role", repo: "/w/ans", branch: "feat/role", brief: "Do it.", provider: "claude", model: null, expectedMtime: 10 }), "start(item, brief): the reviewed text is a parameter, beside the item as it was seen");
check(calls.linkPlace.args.slug === "role" && calls.linkPlace.args.expectedMtime === 9 && calls.setProject.args.expectedMtime === 8, "…and every phase 5 edit carries the plan's mtime");
{
  const rs = fs.readFileSync(here("../../crates/worktrees-core/src/diag.rs"), "utf8");
  check(new RegExp(`EXIT_LAUNCH_REFUSED: i32 = ${P.LAUNCH_REFUSED};`).test(rs), "LAUNCH_REFUSED is core's EXIT_LAUNCH_REFUSED");
  const V = await import(`data:text/javascript;base64,${Buffer.from((await transformWithEsbuild(fs.readFileSync(here("../src/privacypaths.ts"), "utf8"), "privacypaths.ts", { loader: "ts" })).code).toString("base64")}`);
  check(V.privacyArea("~/Documents/x", true) === "Documents" && V.privacyArea("/Users/d/Desktop", true) === "Desktop" && V.privacyArea("/Users/d/downloads/x", true) === "Downloads", "Documents, Desktop and Downloads are warned about");
  check(!V.privacyArea("~/workspace/Documents/x", true) && !V.privacyArea("/Users/d/DocumentsX/x", true) && !V.privacyArea("~/Documents/x", false), "…and nowhere else, and only on a Mac");
}
check(same(P.plansIn([{ id: PL, how: "planned-in" }, { id: PL, how: "planned-in" }, { id: B, how: "related" }]).map((r) => r.id), [PL]),
  "an idea's plans are its planned-in relations, once each");

// ── the windows: one function per act, no second door ──────────────────────
const comp = fs.readFileSync(COMP, "utf8");
const app = fs.readFileSync(APP, "utf8");
const ROWS = ["plan_set_project", "plan_start", "plan_link_place", "plan_make", "plan_add_ideas", "plan_remove_idea", "plan_add_project", "workspace_from_plan", "workspace_set_plan", "workspace_link_idea", "workspace_unlink_idea"];
for (const cmd of ROWS) {
  check(!comp.includes(`"${cmd}"`) && !app.includes(`"${cmd}"`), `neither window invokes ${cmd} except through plans.ts's builder`);
}
check(/if \(target\.kind === "plan"\) \{ void doAddIdeas\(target\.id, target\.ids\)/.test(comp) && /void doAddIdeas\(p, \[r\.id!\]\)/.test(comp),
  "an idea dropped on a plan and the plan strip's Add idea… end in doAddIdeas");
check(/if \(target\.kind === "plan-projects"\) \{ void doAddProject\(target\.id, target\.root\)/.test(comp) && /void doAddProject\(planView\.doc\.id, root\)/.test(comp),
  "a project dropped on the plan's Projects and the Add project… click end in doAddProject");
check((comp.match(/void doMakeWorkspace\(/g) ?? []).length >= 2, "Make workspace: the plan strip and the list row's menu, one function");
check(/doMakePlan\(planSources\(openRow, selected, ideaRowsAll\)\)/.test(comp) && /doMakePlan\(planSources\(null, selected, ideaRowsAll\)\)/.test(comp),
  "Make a plan: from the open idea's strip and from the selection, one function, one rule");
check(/if \(target\.kind === "ws-plan"\) \{ void wsCall\(setPlanCall\(target\.id, target\.plan\)\)/.test(app) && /setPlan: \(plan\) => void wsCall\(setPlanCall\(wsHere, plan\)\),/.test(app),
  "a plan dropped on a workspace and the overview's Link a plan ▸ end in the same call");
check(/if \(target\.kind === "ws-idea"\) \{ void wsCall\(linkIdeaCall\(target\.id, target\.idea\)\)/.test(app) && /linkIdea: \(id\) => void wsCall\(linkIdeaCall\(wsHere, id\)\),/.test(app),
  "an idea dropped on a workspace and the overview's Link an idea ▸ end in the same call");
check(/planOntoWorkspace\(item\.id, onto, all\)/.test(app) && /ideaOntoWorkspace\(item\.id, onto\)/.test(app), "the main window's drop resolves through plans.ts's rules");
const eeHead = app.slice(app.indexOf("function EverythingElseHeader("), app.indexOf("function WsGoneRow("));
const eeGroup = app.slice(app.indexOf('"ws-group ee"'), app.indexOf("})()}", app.indexOf('"ws-group ee"')));
check(eeHead.length > 100 && !/onOpen|openWsOverview|onMenu/.test(eeHead) && eeGroup.length > 100 && !/openWsOverview|onOpen|data-ws-id/.test(eeGroup),
  "Everything Else has no overview to open, no menu, and no data-ws-id a drop could find");
check(/const wsLite = \(\) => \(wsList\?\.workspaces \?\? \[\]\)\.map/.test(app) && /invoke<\{ workspaces: WsOption\[\] \}>\("workspaces_list"\)/.test(comp),
  "what a drop or a Link picker can name is the stored list, which Everything Else is never in");
check(/ideaOntoPlan\(/.test(comp) && /projectOntoPlan\(item\.root, planView\)/.test(comp), "the companion's drop resolves through plans.ts's rules");
check(/<PlanPanel[\s\S]{0,200}view=\{planView\}/.test(comp) && /\{v\.problems\.length > 0 &&/.test(fs.readFileSync(here("../companion/PlanPanel.tsx"), "utf8")),
  "a plan's problems are drawn above the editor, which shows the document whatever they are");

// ── phase 5: the review, the drops, Clone and Create ───────────────────────
const review = fs.readFileSync(START, "utf8"), panel = fs.readFileSync(PANEL, "utf8"), over = fs.readFileSync(OVERVIEW, "utf8") + fs.readFileSync(here("../src/WorkspaceFrame.tsx"), "utf8");
check((comp.match(/startCall\(/g) ?? []).length === 1 && (app.match(/startCall\(/g) ?? []).length === 1, "each window starts in ONE place");
check(!/startCall|doStart|plan_start/.test(panel) && !/startCall|startItem|plan_start/.test(over), "a plan's list starts nothing: its button opens the review");
check(/start=\{\(brief\) => doStart\(/.test(comp) && (comp.match(/doStart\(/g) ?? []).length === 1 && /start=\{\(brief\) => startItem\(/.test(app) && (app.match(/startItem\(/g) ?? []).length === 1,
  "Start is reached through the review alone, with ITS text");
check(/<textarea[^>]*data-testid="start-brief"[^>]*value=\{brief\}/.test(review) && /props\.start\(brief\)/.test(review) && (review.match(/props\.start\(/g) ?? []).length === 1, "the review shows the brief it sends, once");
check(/if \(running\.current\) return;/.test(review) && /disabled=\{busy \|\| !!problem\}/.test(review), "a second press while it runs, or a refused item, starts nothing");
check(/\n            readOnly\n/.test(app.slice(app.indexOf("<StartReview"), app.indexOf("/>", app.indexOf("<StartReview")))) && !/\breadOnly\b/.test(comp.slice(comp.indexOf("<StartReview"), comp.indexOf("/>", comp.indexOf("<StartReview")))),
  "the brief is edited in the Ideas window and read-only in the overview");
check(/if \(target\.kind === "work"\) \{ setPlaceFor\(null\); void doLinkPlace\(/.test(comp) && /setPlaceFor\(null\); void doLinkPlace\(planView\.doc\.id, w, root, slug\)/.test(comp), "a place dropped on an item and Link a place… end in doLinkPlace (Ideas)");
check(/if \(target\.kind === "work"\) \{ void linkPlaceToItem\(/.test(app) && /setWorkPick\(null\); void linkPlaceToItem\(w, root, slug\)/.test(app), "…and in linkPlaceToItem (overview)");
check(/placeOntoItem\(item\.root, item\.slug, w, itemPlace\(/.test(comp) && /placeOntoItem\(item\.repo, item\.slug, it\.item, it\.at, ws\)/.test(app), "both resolve through plans.ts's rule");
check(app.indexOf("closest<HTMLElement>('[data-drop=\"work\"]')") > 0 && app.indexOf("closest<HTMLElement>('[data-drop=\"work\"]')") < app.indexOf("closest<HTMLElement>('[data-drop=\"mention\"]')"), "a work item is hit-tested before the terminal");
check(!/clone_project|create_project/.test(comp + panel + over), "no second clone or create path: neither the Ideas window nor the overview has one");
check((app.match(/"clone_project"/g) ?? []).length === 1 && (app.match(/"create_project"/g) ?? []).length === 1, "the main window's dialogs stay the one door to each");
check(/commitWs\(after\);\s*setNpOpen\(false\);\s*await planArrive\(npSeed, newRoot\(before, after\)\)/.test(app) && /setCloneOpen\(false\);\s*await planArrive\(seed, done\.root\)/.test(app), "a plan's project, once here, is marked in its plan");
check(/<PrivacyNote path=\{full\} \/>/.test(app) && /<PrivacyNote path=\{`\$\{trimmedParent\}\/\$\{folder\}`\} \/>/.test(app), "both dialogs warn on a guarded folder");
{
  const ops = fs.readFileSync(here("../../crates/worktrees-api/src/ops.rs"), "utf8"), api = fs.readFileSync(here("../../crates/worktrees-api/src/plans.rs"), "utf8");
  const body = ops.slice(ops.indexOf("pub fn new_place_with("), ops.indexOf("pub fn switch_place("));
  check(/\.value_opt\("--brief", brief\)/.test(body) && /args\.call\(p, ui, ops::cmd_new\)/.test(body) && /new_place_with\(ctx, r\.root\.clone\(\), [^;]*Some\(&r\.brief\)\)/.test(api), "a Start goes down `new --brief`'s own path, with the reviewed text");
  check(/fn new_place\([\s\S]{0,400}?new_place_with\(ctx, repo, branch, base, name, provider, model, force, None\)/.test(ops) && !/brief/.test(rowLine("new_place") ?? "x"), "new_place, which a device holds, still takes no brief");
}
check(/askMain\(\{ act: "project", plan, name \}\)/.test(comp) && /invoke<PlanView>\("plan_read", \{ id: a\.plan \}\)/.test(app), "the Ideas window names the project; the main window reads the plan itself");

// ── a press can START each drag, and Home leaves the overview ───────────────
// `arm` (navdrag.ts) refuses a press inside a button that is not marked
// `data-drag-handle`, so a chip without it drags nothing and says nothing.
check(/closest\("button,input,textarea,select,a"\);\s*if \(ctl && !ctl\.hasAttribute\("data-drag-handle"\)\) return;/.test(fs.readFileSync(here("../src/navdrag.ts"), "utf8")),
  "navdrag: a press in a button arms a drag only when the button is a data-drag-handle");
for (const [file, text, want] of [
  ["WorkspaceOverview.tsx", fs.readFileSync(OVERVIEW, "utf8"), 3],
  ["WorkspaceFrame.tsx", fs.readFileSync(here("../src/WorkspaceFrame.tsx"), "utf8"), 1],
  ["PlanPanel.tsx", fs.readFileSync(PANEL, "utf8"), 1],
  ["StartWork.tsx", review, 1],
]) {
  const tags = [...text.matchAll(/onPointerDown=\{\(e\) => props\.arm(?:\?\.)?\(/g)].map((m) => text.slice(text.lastIndexOf("<button", m.index), m.index));
  check(tags.length === want, `${file}: ${want} drag source(s) (${tags.length})`);
  tags.forEach((t, i) => check(/^<button\b/.test(t) && /\sdata-drag-handle=""/.test(t), `${file}: drag source ${i + 1} is a data-drag-handle`));
}
const homeItem = app.match(/<button className=\{"home-item"[^\n]*/)?.[0] ?? "";
check(/setSel\(null\);/.test(homeItem) && /setWsOpen\(null\);/.test(homeItem), "Home in the nav leaves an open overview, not only a place");
const homeChord = app.match(/if \(target === "home"\) \{[^\n]*\}/)?.[0] ?? "";
check(/setSel\(null\);/.test(homeChord) && /setWsOpen\(null\);/.test(homeChord), "…and so does ⌘1");

// ── the mock answers every phase 4 and 5 row ──────────────────────────────────────
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
const ask = (o, t = now) => P.mainAsk(JSON.stringify({ at: now, ...o }), t);
check(same(ask({ act: "project", plan: PL, name: "ans" }), { act: "project", plan: PL, name: "ans" }) && same(ask({ act: "place", root: R, slug: "role" }), { act: "place", root: R, slug: "role" }), "the Ideas window's asks are read");
check(ask({ act: "project", plan: "../x", name: "a" }) === null && ask({ act: "project", plan: PL }) === null && ask({ act: "place", root: "rel", slug: "x" }) === null && ask({ act: "start", plan: PL }) === null
  && ask({ act: "project", plan: PL, name: "a" }, now + P.SHOW_TTL_MS + 1) === null && P.mainAsk("{", now) === null, "…and a malformed, unknown or stale one is ignored");
check(/window\.localStorage\.removeItem\(MAIN_ASK_KEY\)/.test(app), "the main window takes an ask once");
check(/take\(windowEnv\(\), FLOAT\)/.test(comp) && /take\(windowEnv\(\), DOCK\)/.test(app) && /window\.localStorage\.removeItem\(WS_SHOW_KEY\)/.test(app),
  "each window takes a request once (removes the key it read)");

console.log(failed ? `\nplans-check: ${failed} failure(s)` : "\nplans-check: all ok");
process.exit(failed ? 1 : 0);
