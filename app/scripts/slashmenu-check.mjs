// The chat composer's slash commands (docs/proposals/chat-slash-commands.md):
// the menu's filtering, the two-press confirm, and the refusals in words —
// `app/src/slash.ts`, run as it is (its only import is a type) — and that
// the words key on the refusals the host actually gives:
//
//   node app/scripts/slashmenu-check.mjs      # exits non-zero on failure
//
// SLASHMENU_{SLASH,AGENTS,CORE} point the check at a mutated copy.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = {
  slash: process.env.SLASHMENU_SLASH ?? here("../src/slash.ts"),
  agents: process.env.SLASHMENU_AGENTS ?? here("../../crates/worktrees-api/src/agents.rs"),
  core: process.env.SLASHMENU_CORE ?? here("../../crates/worktrees-core/src/slash.rs"),
};
let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const check = (what, got, want) => (eq(got, want) ? ok(what) : fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`));
const read = (p) => fs.readFileSync(p, "utf8");

const src = read(SRC.slash);
if (/^import (?!type )/m.test(src)) { fail("slash.ts imports at runtime — it must stay pure"); process.exit(1); }
const js = (await transformWithEsbuild(src, "slash.ts", { loader: "ts", format: "esm" })).code;
const S = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

const cmd = (name, o = {}) => ({ name, description: o.description ?? `${name} it`, source: o.source ?? "built-in", tier: o.tier ?? "normal", args: o.args ?? true, ...o });
const LIST = [
  // `statusline` before `status`, as no alphabet puts them: the exact name
  // must come first by the menu's own order.
  cmd("statusline"), cmd("status", { description: "Show Claude Code status" }), cmd("usage", { description: "Show session cost, plan usage" }),
  cmd("compact"), cmd("context"), cmd("model", { args: false }), cmd("clear", { confirm: "Starts a new conversation." }),
  cmd("login", { tier: "sensitive", why: "account" }), cmd("hello", { source: "project" }), cmd("sub:nested", { source: "project" }),
  cmd("preexec", { source: "project", tier: "sensitive", why: "its file runs a shell command before the model sees it" }),
];
const names = (l) => l.map((c) => c.name);

// ── the draft ────────────────────────────────────────────────────────────────
check("`/status` is a command line", S.slashDraft("/status"), { name: "status", args: null });
check("a space starts its argument", S.slashDraft("/compact keep the facts"), { name: "compact", args: "keep the facts" });
check("a leading space is a message", S.slashDraft(" /status"), null);
check("plain text is a message", S.slashDraft("hello /status"), null);
check("the menu shows while the name is typed", [S.menuFor("/"), S.menuFor("/sta"), S.menuFor("/compact "), S.menuFor("hi")], ["", "sta", null, null]);

// ── the menu's filtering ─────────────────────────────────────────────────────
check("names that start with it, the exact one first", names(S.filterCommands(LIST, "status")), ["status", "statusline"]);
check("then names that contain it, then descriptions", names(S.filterCommands(LIST, "us")), ["usage", "status", "statusline"]);
check("case does not matter", names(S.filterCommands(LIST, "COMP")), ["compact"]);
check("a namespace's part matches", names(S.filterCommands(LIST, "nest")), ["sub:nested"]);
check("a description matches", names(S.filterCommands(LIST, "plan usage")), ["usage"]);
check("not from fewer than three letters: `co` lists only names", names(S.filterCommands(LIST, "co")), ["compact", "context"]);
check("`/` alone lists them all, shortest first", S.filterCommands(LIST, "").length, LIST.length);
check("nothing matches nothing", S.filterCommands(LIST, "zzz"), []);
const many = Array.from({ length: 80 }, (_, i) => cmd(`c${String(i).padStart(2, "0")}`));
check("the menu is capped", S.filterCommands(many, "").length, S.MENU_MAX);
check("sources in words", LIST.filter((c) => ["status", "hello"].includes(c.name)).map(S.sourceWord).concat(S.sourceWord(cmd("x", { source: "plugin", plugin: "worktrees" }))), ["Built-in", "This repository", "Plugin · worktrees"]);

// ── the confirm ──────────────────────────────────────────────────────────────
const by = (n) => LIST.find((c) => c.name === n);
check("a plain command has no confirm", S.confirmWords(by("status")), null);
check("/clear confirms with its own words", S.confirmWords(by("clear")), "Starts a new conversation.");
check("a sensitive command confirms with why", S.confirmWords(by("preexec")), "A sensitive command: its file runs a shell command before the model sees it.");
check("a plain command runs on one press", S.confirmStep(by("status"), "/status", null, 1000), { run: true, armed: null });
const first = S.confirmStep(by("clear"), "/clear", null, 1000);
check("the first press arms", first, { run: false, armed: { line: "/clear", until: 1000 + S.ARM_MS } });
check("the second press of the same line runs", S.confirmStep(by("clear"), "/clear", first.armed, 2000), { run: true, armed: null });
check("a second press too late arms again", S.confirmStep(by("clear"), "/clear", first.armed, 1000 + S.ARM_MS + 1).run, false);
check("a changed line arms again", S.confirmStep(by("login"), "/login", first.armed, 2000).run, false);

// ── the refusals the composer makes before the host is asked ─────────────────
const argProblem = (s) => (/[\u0000-\u001f]/.test(s) ? "Text contains a control character" : null);
check("a listed command with no argument is fine", S.commandProblem(LIST, "/status", argProblem), null);
check("an unknown name", S.commandProblem(LIST, "/nope", argProblem), "/nope is not one of this agent’s commands.");
check("/model takes no argument", S.commandProblem(LIST, "/model sonnet", argProblem)?.startsWith("/model takes no argument from the chat"), true);
check("no @ in an argument", S.commandProblem(LIST, "/compact keep @README", argProblem)?.includes("“@”"), true);
check("the argument's line rule is the message rule", S.commandProblem(LIST, "/compact a\tb", argProblem), "Its argument: text contains a control character");
check("before the list is read", S.commandProblem(null, "/status", argProblem), "Reading the agent’s commands…");
check("a message is no command", S.commandProblem(LIST, "hello", argProblem), null);

// ── the words, and the host's refusals they key on ───────────────────────────
const agents = read(SRC.agents);
const core = read(SRC.core);
const rust = (src, name) => src.match(new RegExp(`pub const ${name}: &str = "([^"]*)";`))?.[1] ?? null;
const HOST = {
  admin: rust(agents, "ERR_NEEDS_ADMIN"), menu: rust(agents, "ERR_MENU_OTHER"), promptUp: rust(agents, "ERR_PROMPT_UP"),
  unknown: rust(core, "ERR_UNKNOWN"), shell: rust(core, "ERR_SHELL"), memory: rust(core, "ERR_MEMORY"), noArgs: rust(core, "ERR_NO_ARGS"),
};
for (const [k, v] of Object.entries(HOST)) if (!v) fail(`the host has no ${k} refusal where this check reads it`);
const said = (err) => S.commandWords(err, "Claude");
check("admin", said(HOST.admin).line.includes("admin permission"), true);
check("unknown", said(`${HOST.unknown} /nope`).line, "Not one of this agent’s commands: /nope.");
check("shell", said(HOST.shell).line.startsWith("“!” runs a shell"), true);
check("memory", said(HOST.memory).line.startsWith("“#” writes"), true);
check("no argument", said(`/model ${HOST.noArgs}: pick from what it opens instead`).line.startsWith("/model takes no argument"), true);
check("another name lit: the terminal", said(`${HOST.menu} (/sub:nested) than /nested: nothing was submitted`).terminal, true);
check("a prompt is up", said(HOST.promptUp).prompt, true);
check("an older host", said("unknown command: agent_command").line.includes("update Worktrees"), true);

console.log(failed ? `\nslashmenu-check: ${failed} failure(s)` : "\nslashmenu-check: all good");
process.exit(failed ? 1 : 0);
