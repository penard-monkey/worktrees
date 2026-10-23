// The agent rows — `agent_prompt`, `agent_answer`, `agent_send`,
// `agent_turns` (iOS MVP §2.4, S6–S11) — say the same thing in the four places
// that state them:
//
//   node app/scripts/agent-rows-check.mjs      # exits non-zero on failure
//
// 1. serve's dispatch table (`crates/worktrees-cli/src/serve/dispatch.rs`):
//    each row's capability and objects, PINNED here, because a one-word edit
//    there (`answer` → `write`) would drop the step-up and the drive-class
//    audit budget that `answer` and `instruct` buy, with every Rust test of
//    the body still green. Every `agent_*` row is one of the four, and each
//    body is its `agents::` function.
// 2. the api (`crates/worktrees-api/src/agents.rs`): the refusal strings a
//    client keys on — `the prompt changed` (the compare-and-swap) and `no such
//    choice` — are the ones the docs and the mock use.
// 3. the mock (`app/src/mock/install.ts`): it answers all four, and refuses
//    with those same strings, so a client developed against the harness meets
//    the words it will meet on a host.
// 4. the docs (`docs/proposals/ios-mvp.md`, §2.4's "As built" table): each
//    row's spec, spelled as the table spells it.
// And the remote door (`app/src/ipc.ts`) refuses none of them locally.
//
// AGENT_ROWS_{TABLE,API,MOCK,DOC,IPC} point the check at a mutated copy.
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = {
  table: process.env.AGENT_ROWS_TABLE ?? here("../../crates/worktrees-cli/src/serve/dispatch.rs"),
  api: process.env.AGENT_ROWS_API ?? here("../../crates/worktrees-api/src/agents.rs"),
  mock: process.env.AGENT_ROWS_MOCK ?? here("../src/mock/install.ts"),
  doc: process.env.AGENT_ROWS_DOC ?? here("../../docs/proposals/ios-mvp.md"),
  ipc: process.env.AGENT_ROWS_IPC ?? here("../src/ipc.ts"),
};

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const read = (p) => fs.readFileSync(p, "utf8");

// What each row must be: `[capability, objects…]` exactly as the table writes it.
const ROWS = {
  agent_prompt: "read, place(repo, slug)",
  agent_answer: "answer, place(repo, slug)",
  agent_send: "instruct, place(repo, slug), session_of(session, repo, slug)",
  agent_turns: "read, place(repo, slug), opt(session_of(session, repo, slug))",
};
const norm = (s) => s.replace(/\s+/g, " ").trim();

// ── 1. the table ─────────────────────────────────────────────────────────────
// Rows sit at four spaces inside `commands! { cx, a;`; a body may run over
// several lines, so a row's text is everything up to the next row.
const src = read(SRC.table);
const open = src.indexOf("commands! { cx, a;");
const close = open < 0 ? -1 : src.indexOf("\n}\n", open);
if (open < 0 || close < 0) fail("the `commands! { cx, a; … }` block is not where this check looks");
const block = open < 0 ? "" : src.slice(open, close);
const starts = [...block.matchAll(/^ {4}([a-z][a-z0-9_]*): (\w+) (?:\[([^\]]*(?:\([^)]*\)[^\]]*)*)\] )?=>/gm)];
const rowText = new Map(starts.map((m, i) => [m[1], block.slice(m.index, starts[i + 1]?.index ?? block.length)]));
const spec = new Map(starts.map((m) => [m[1], { exposure: m[2], spec: norm(m[3] ?? "") }]));
if (starts.length < 150) fail(`only ${starts.length} rows parsed from the table — the row pattern no longer matches`);
for (const [name, want] of Object.entries(ROWS)) {
  const got = spec.get(name);
  if (!got) fail(`${name} is not a row of the dispatch table`);
  else if (got.exposure !== "Engine") fail(`${name} is ${got.exposure}, expected Engine (any client with the capability may call it)`);
  else if (got.spec !== want) fail(`${name} is [${got.spec}], expected [${want}]`);
  else ok(`${name}: Engine [${want}]`);
  if (got && !rowText.get(name).includes(`agents::${name}(`)) fail(`${name}'s body is not agents::${name}`);
}
const extra = [...spec.keys()].filter((n) => n.startsWith("agent_") && !(n in ROWS) && !["agent_models", "agent_setup_status", "agent_setup_fix", "agent_link_skills", "agent_user_skills", "agent_guidance_status", "agent_guidance_diff", "agent_guidance_merge"].includes(n));
if (extra.length) fail(`agent rows this check does not know: ${extra.join(", ")} — pin them in ROWS`);
else ok("no other agent-verb row in the table");

// ── 2. the api's refusal strings ─────────────────────────────────────────────
const api = read(SRC.api);
const constOf = (name) => api.match(new RegExp(`pub const ${name}: &str = "([^"]*)";`))?.[1];
const ERR = { changed: constOf("ERR_CHANGED"), noChoice: constOf("ERR_NO_CHOICE"), promptUp: constOf("ERR_PROMPT_UP") };
if (ERR.changed !== "the prompt changed") fail(`agents.rs ERR_CHANGED is ${JSON.stringify(ERR.changed)}, expected "the prompt changed" (the CAS refusal clients key on)`);
else ok('the compare-and-swap refuses with "the prompt changed"');
if (ERR.noChoice !== "no such choice") fail(`agents.rs ERR_NO_CHOICE is ${JSON.stringify(ERR.noChoice)}, expected "no such choice"`);
else ok('a choice the prompt does not have is "no such choice"');
if (!ERR.promptUp?.includes("agent_answer")) fail("agents.rs ERR_PROMPT_UP does not point the client at agent_answer");
else ok("a send while a prompt is up names agent_answer");

// ── 3. the mock ──────────────────────────────────────────────────────────────
const mock = read(SRC.mock);
const missing = Object.keys(ROWS).filter((n) => !mock.includes(`case "${n}":`));
if (missing.length) fail(`mock/install.ts does not answer: ${missing.join(", ")}`);
else ok("the mock answers all four rows");
for (const s of [ERR.changed, ERR.noChoice, ERR.promptUp].filter(Boolean)) {
  if (!mock.includes(`throw ${JSON.stringify(s)}`)) fail(`the mock never refuses with ${JSON.stringify(s)}`);
  else ok(`the mock refuses with ${JSON.stringify(s)}`);
}

// ── 3b. the remote door (app/src/ipc.ts, #555) ───────────────────────────────
// A window driving a paired host refuses some rows LOCALLY, before the host
// is asked: CONSOLE_ROWS always, ADMIN_ROWS without `admin`, and it routes
// HOST_PATH_LOCAL and REMOTE_SETTINGS elsewhere. The agent rows are read,
// answer and instruct: a device holding those reaches them through the door,
// and serve decides (step-up, the audit budget). None may sit in those lists.
const ipc = read(SRC.ipc);
// The list's whole literal, keys included (REMOTE_SETTINGS is an object).
const listed = (name) => ipc.match(new RegExp(`export const ${name}[^=]*=\\s*([\\[{][\\s\\S]*?[\\]}]);`))?.[1] ?? null;
for (const list of ["CONSOLE_ROWS", "ADMIN_ROWS", "HOST_PATH_LOCAL", "REMOTE_SETTINGS"]) {
  const block = listed(list);
  if (!block) { fail(`ipc.ts has no ${list} for this check to read`); continue; }
  const caught = Object.keys(ROWS).filter((n) => new RegExp(`\\b${n}\\b`).test(block));
  if (caught.length) fail(`ipc.ts's ${list} holds ${caught.join(", ")} — a remote window would never ask the host`);
  else ok(`the remote door's ${list} leaves the agent rows to the host`);
}

// ── 4. the docs ──────────────────────────────────────────────────────────────
// A row of the "As built" table: | `agent_prompt` | `[read, place(repo, slug)]` | …
const doc = read(SRC.doc);
for (const [name, want] of Object.entries(ROWS)) {
  const m = doc.match(new RegExp("^\\|\\s*`" + name + "`\\s*\\|\\s*`\\[([^`]*)\\]`", "m"));
  if (!m) fail(`docs/proposals/ios-mvp.md has no "As built" row for ${name}`);
  else if (norm(m[1]) !== want) fail(`the docs say ${name} is [${norm(m[1])}], the table says [${want}]`);
  else ok(`the docs agree on ${name}`);
}

console.log(failed ? `\nagent-rows-check: ${failed} failure(s)` : "\nagent-rows-check: all good");
process.exit(failed ? 1 : 0);
