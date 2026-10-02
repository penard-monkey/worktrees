// Guards the nav drag's TERMINAL drop — what dropping a place onto an agent's
// pane types, and when it refuses (cross-project proposal §6).
//
//   node app/scripts/drop-check.mjs        # exits non-zero on failure
//
// `dnd.ts::mentionPlan` is the frontend's half of core's `reach::plan_drop`
// (crates/worktrees-core/src/reach.rs): it words the drag chip and the refusal
// BEFORE the drop, and the backend decides again on drop. A frontend that
// mirrors a core decision lies quietly unless something compares the two
// (AGENTS.md), so this runs the REAL `mentionPlan` (same slice-the-source
// shape as dnd-check.mjs) over the cases core's own unit test pins, and fails
// if that test stops pinning them.
//
// It also pins the wiring that made the old drop Claude-only and same-project
// only: every harness's pane is a drop target, and the drop passes the
// receiving project and the pane's provider to `drop_reference`.

import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

const js = (await transformWithEsbuild(read("../src/dnd.ts"), "dnd.ts", { loader: "ts", format: "esm" })).code;
const { mentionPlan } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// A base input: alpha → alpha, Claude up, reach off, both registered.
const base = {
  fromRepo: "/w/alpha", intoRepo: "/w/alpha", provider: "claude", agentUp: true, reach: "off",
  fromPrivate: false, intoPrivate: false, fromRegistered: true, intoRegistered: true, waiting: false,
};
const plan = (p) => mentionPlan({ ...base, ...p });
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// ── 1. the cases core's `a_drop_is_a_token_only_within_a_project_into_claude` pins ──
const cases = [
  ["same project into Claude → the @-token, no reach needed", {}, { ok: true, foreign: false, token: true }],
  ["same project into Codex → an address, no reach needed", { provider: "codex" }, { ok: true, foreign: false, token: false }],
  ["same project into pi, unregistered → an address", { provider: "pi", fromRegistered: false, intoRegistered: false }, { ok: true, foreign: false, token: false }],
  ...["claude", "codex", "pi"].map((h) => [`another project into ${h}, reach on → an address`, { fromRepo: "/w/beta", provider: h, reach: "read" }, { ok: true, foreign: true, token: false }]),
];
for (const [name, p, want] of cases) {
  const got = plan(p);
  if (!same(got, want)) fail(`${name}: got ${JSON.stringify(got)}`);
  else ok(name);
}
const refusals = [
  ["reach off", { fromRepo: "/w/beta" }, /Settings/],
  ["dragged project private", { fromRepo: "/w/client", reach: "read", fromPrivate: true }, /private/],
  ["receiving project private", { fromRepo: "/w/beta", reach: "read", intoPrivate: true }, /private/],
  ["dragged project unregistered", { fromRepo: "/w/nope", reach: "read", fromRegistered: false }, /not registered/],
  ["receiving project unregistered", { fromRepo: "/w/beta", reach: "read", intoRegistered: false }, /not registered/],
];
for (const [name, p, re] of refusals) {
  const got = plan(p);
  if (!got || got.ok || !re.test(got.hint)) fail(`refuse when ${name}: got ${JSON.stringify(got)}`);
  else ok(`refuses when ${name}`);
}
// Core must still pin the same rule, or this table is testing a memory.
const core = read("../../crates/worktrees-core/src/reach.rs");
const t = core.slice(core.indexOf("fn a_drop_is_a_token_only_within_a_project_into_claude"));
if (!t || !/Drop::Token/.test(t) || !/"codex"\)\.unwrap\(\), Drop::Address/.test(t) || !/for h in \["claude", "codex", "pi"\]/.test(t)) {
  fail("reach.rs no longer pins the token/address rule this table mirrors (a_drop_is_a_token_only_within_a_project_into_claude)");
} else ok("core's plan_drop test still pins the rule mirrored here");
for (const [needle, what] of [["Level::Off", "reach off"], ["from.private", "dragged project private"], ["into.private", "receiving project private"]]) {
  const body = core.slice(core.indexOf("pub fn plan_drop"), core.indexOf("#[derive", core.indexOf("pub fn plan_drop")));
  if (!body.includes(needle)) fail(`core plan_drop no longer refuses on ${what} — the mirror would refuse a drop the backend allows`);
}

// ── 2. the waiting-pane refusal: Codex and pi only ─────────────────────────
for (const h of ["codex", "pi"]) {
  const got = plan({ provider: h, waiting: true });
  if (!got || got.ok || !/waiting/.test(got.hint)) fail(`a waiting ${h} pane must refuse: ${JSON.stringify(got)}`);
  else ok(`a waiting ${h} pane refuses the drop`);
}
if (!same(plan({ waiting: true }), { ok: true, foreign: false, token: true })) fail("a waiting CLAUDE pane must still take the drop (its prompt ignores a bracketed paste)");
else ok("a waiting Claude pane still takes it");
if (plan({ agentUp: false }) !== null) fail("no agent in that pane → not a drop target");
else ok("no agent up → not a drop target");
if (plan({ fromRepo: "/w/beta", reach: null }) !== null) fail("reach not read yet → no promise either way");
else ok("reach unknown → no promise");
const lib = read("../src-tauri/src/lib.rs");
const dr = lib.slice(lib.indexOf("async fn drop_reference("), lib.indexOf("fn drop_token("));
if (!/State::Waiting/.test(dr) || !/prov\.id != worktrees_core::provider::CLAUDE\.id/.test(dr)) {
  fail("drop_reference no longer refuses a waiting Codex/pi pane itself — the frontend's refusal is advice, the backend's is the gate");
} else ok("drop_reference refuses a waiting Codex/pi pane itself");

// ── 3. the wiring ──────────────────────────────────────────────────────────
const term = read("../src/TerminalPane.tsx");
if (/drop=\{provider === "claude"/.test(term) || !/dropProvider=\{provider\}/.test(term)) {
  fail("TerminalPane must make EVERY harness's pane a drop target and say which harness it is");
} else ok("every harness's pane is a drop target, labelled with its provider");
const app = read("../src/App.tsx");
if (!/mentionPlan\(\{/.test(app)) fail("resolveDrop no longer asks mentionPlan");
else ok("resolveDrop asks mentionPlan");
const call = app.match(/invoke<string>\("drop_reference", \{([^}]*)\}/);
if (!call || !/intoRepo/.test(call[1]) || !/provider/.test(call[1])) {
  fail("commitDrop must pass intoRepo and provider to drop_reference — the server name is the RECEIVING project's");
} else ok("drop_reference gets the receiving project and the pane's provider");
if (/a session can only reference worktrees from its own project/.test(app)) fail("the old blanket refusal is back");
else ok("the old same-project-only refusal is gone");

console.log(bad ? `\n${bad} failure(s)` : "\nall good");
process.exit(bad ? 1 : 0);
