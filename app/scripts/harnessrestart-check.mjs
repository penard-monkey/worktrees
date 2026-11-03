// Deterministic check of what Settings says and offers about agent sessions
// that run an older Claude or Codex (`harnessrestart.ts`,
// docs/proposals/harness-restart.md).
//
// The host decides which sessions are older and what each is doing; the page
// counts them and words two confirms. Those words are the whole safety story
// of the force path — it interrupts turns and cancels open questions, and the
// (main) orchestrator may be among them — so a miscount here restarts work
// the person was told would be left alone. It evaluates the REAL module
// source, as updatepoll-check.mjs does.
//
//   node harnessrestart-check.mjs [path/to/harnessrestart.ts]   exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const SRC = process.argv[2] || fileURLToPath(new URL("../src/harnessrestart.ts", import.meta.url));
const raw = fs.readFileSync(SRC, "utf8");
if (/^\s*import\s(?!type\b)/m.test(raw)) throw new Error(`${SRC}: must stay import-free (see header)`);
const js = (await transformWithEsbuild(raw, "harnessrestart.ts", { loader: "ts", format: "esm" })).code;
const M = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const ok = (cond, msg) => { if (!cond) { failed++; console.error("not ok", msg); } else console.log("ok  ", msg); };
const eq = (got, want, msg) => ok(JSON.stringify(got) === JSON.stringify(want), `${msg}${JSON.stringify(got) === JSON.stringify(want) ? "" : ` — got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`}`);

const row = (project, slug, state, version = "2.1.316", why) =>
  ({ project: `/p/${project}`, project_name: project, slug, session: `${project}-${slug}`, version, state, ...(why ? { why } : {}) });
const claude = (sessions, outside = []) => ({ tool: "claude", installed_version: "2.1.318", sessions, outside });

// ── selection ────────────────────────────────────────────────────────────────
const mixed = claude([
  row("worktrees", "(main)", "busy"),
  row("orfis", "(main)", "waiting"),
  row("audiorb", "lane", "idle"),
  row("crate", "(main)", "idle"),
  row("site", "fix", "busy"),
  row("ios", "(main)", "idle", "2.1.309", "its session file names no conversation, so it cannot be resumed exactly"),
], [{ version: "2.1.310" }, { version: "2.1.316" }]);
const p = M.restartPlan(mixed);
eq([p.older, p.idle, p.busy, p.waiting, p.unresumable], [6, 2, 2, 1, 1], "idle, busy and waiting are counted apart; an unresumable one is in none");
eq(p.activeMains, ["worktrees", "orfis"], "the (main)s a force restart would interrupt are named");
ok(!p.activeMains.includes("ios"), "an unresumable (main) is never named as restarted");
ok(M.showInactive(p) && M.showForce(p), "both buttons when some are idle and some active");

eq(M.inactiveLabel(p), "Restart 2 inactive", "the inactive button counts idle sessions only");
eq(M.forceLabel(p), "Force restart all 5", "the force button counts every restartable session");
eq(M.inactiveConfirm("Claude", p), "This restarts 2 open Claude sessions. Each resumes its conversation.", "the inactive confirm");
eq(
  M.forceConfirm("Claude", p),
  "This restarts all 5 Claude sessions, and also interrupts 2 sessions mid-turn and cancels 1 open question. Among them: the (main) agent of worktrees and orfis. Each resumes its conversation.",
  "the force confirm says what it interrupts and cancels, and names the (main)s",
);

const allIdle = M.restartPlan(claude([row("a", "x", "idle"), row("b", "(main)", "idle")]));
ok(M.showInactive(allIdle) && !M.showForce(allIdle), "no force button when force would do nothing more");
const allBusy = M.restartPlan(claude([row("a", "(main)", "busy")]));
ok(!M.showInactive(allBusy) && M.showForce(allBusy), "no inactive button when nothing is idle");
eq(M.forceConfirm("Claude", allBusy), "This restarts all 1 Claude session, and also interrupts 1 session mid-turn. Among them: the (main) agent of a. Each resumes its conversation.", "one busy (main)");
const none = M.restartPlan(claude([]));
ok(!M.showInactive(none) && !M.showForce(none), "nothing older, no buttons");
ok(!M.showInactive(M.restartPlan(null)), "no answer yet, no buttons");

// ── the summary line ─────────────────────────────────────────────────────────
eq(M.summaryLine("Claude", claude([row("a", "x", "idle"), row("b", "y", "busy")])), "2 Claude sessions run 2.1.316; 2.1.318 is installed.", "one older version");
eq(M.summaryLine("Claude", mixed), "6 Claude sessions run older versions (2.1.309, 2.1.316); 2.1.318 is installed.", "several, oldest first");
eq(M.summaryLine("Claude", claude([row("a", "x", "idle", "2.1.316")])), "1 Claude session runs 2.1.316; 2.1.318 is installed.", "singular");
eq(
  M.summaryLine("Codex", { tool: "codex", installed_version: "0.160.1", sessions: [row("a", "x", "idle", null)], outside: [] }),
  "1 Codex session started before Codex 0.160.1 was installed.",
  "a Codex whose build is unknown",
);
eq(M.summaryLine("Claude", claude([])), "", "nothing older, no line");
eq(M.outsideLine("Claude", mixed), "2 more Claude sessions outside Worktrees (an editor or a terminal) run an older version: restart them by hand.", "outside sessions, by hand");
eq(M.outsideLine("Claude", claude([], [{ version: "2.1.1" }])), "1 more Claude session outside Worktrees (an editor or a terminal) runs an older version: restart it by hand.", "one outside");
eq(M.outsideLine("Claude", claude([])), "", "none outside, no line");

// ── results ──────────────────────────────────────────────────────────────────
const r = (outcome, extra = {}) => ({ project: "/p/w", project_name: "w", slug: "(main)", session: "w-main", outcome, ...extra });
eq(M.resultLine(r("restarted", { version: "2.1.318" })), "w · (main): restarted, now 2.1.318", "a proved restart names its version");
eq(M.resultLine(r("restarted", { why: "its new version did not show within 20 s" })), "w · (main): restarted (its new version did not show within 20 s)", "an unproved restart says so");
eq(M.resultLine(r("skipped", { why: "it turned busy since the list was read" })), "w · (main): skipped — it turned busy since the list was read", "a skip says why");
eq(M.resultLine(r("failed", { why: "tmux respawn-pane exited 1" })), "w · (main): failed — tmux respawn-pane exited 1", "a failure says why");
eq(M.tallyLine({ restarted: 2, skipped: 1, failed: 0 }), "2 restarted · 1 skipped", "the tally leaves out zeroes");

if (failed) {
  console.error(`\nharnessrestart-check: ${failed} failed`);
  process.exit(1);
}
console.log("\nharnessrestart-check: all good");
