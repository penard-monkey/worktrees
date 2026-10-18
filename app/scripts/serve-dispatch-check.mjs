// Every command the app registers must be dispatchable over the socket, and
// nothing the socket dispatches may be a command the app does not have.
//
//   node app/scripts/serve-dispatch-check.mjs      # exits non-zero on failure
//
// `worktrees serve` answers the same `{cmd, args}` the frontend's `invoke`
// sends, out of one table in `crates/worktrees-cli/src/serve/dispatch.rs` that
// pairs each command with its exposure. A command added to the shim's
// `generate_handler!` without a row there is a request that fails with
// `unknown command` — correct, loud, and only discovered when a phone presses
// the button. A row with no handler behind it is a command the desktop cannot
// make and the socket can. Both are drift between two lists that nothing
// compiles together, which is the api-boundary-check family's job to hold.
//
// Parsed from the sources rather than from a build: the table is a macro
// invocation and the handler list is a macro argument, and neither is a Rust
// item a test could enumerate.
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const SHIM = fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url));
const TABLE = fileURLToPath(new URL("../../crates/worktrees-cli/src/serve/dispatch.rs", import.meta.url));

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);

// ── the shim's handler list ──────────────────────────────────────────────────
const lib = fs.readFileSync(SHIM, "utf8");
const gh = /tauri::generate_handler!\[([\s\S]*?)\]\)/.exec(lib);
if (!gh) fail("generate_handler![…] not found in the shim");
const handlers = (gh?.[1] ?? "").split(",").map((s) => s.trim()).filter(Boolean);
if (handlers.length === 0) fail("the handler list parsed as empty");
else ok(`${handlers.length} commands registered in the shim`);

// ── the dispatcher's table ───────────────────────────────────────────────────
// Rows are `    name: Exposure => …` at exactly four spaces of indent inside
// the `commands! { cx, a;` invocation; bodies are indented deeper, so a line
// inside one cannot be mistaken for a row.
const src = fs.readFileSync(TABLE, "utf8");
const open = src.indexOf("commands! { cx, a;");
const close = open < 0 ? -1 : src.indexOf("\n}\n", open);
if (open < 0 || close < 0) fail("the `commands! { cx, a; … }` block is not where this check looks");
const block = open < 0 ? "" : src.slice(open, close);
const rows = [...block.matchAll(/^ {4}([a-z][a-z0-9_]*): (Engine|ClientLocal|HostAdmin|Stream) =>/gm)];
const table = rows.map((m) => m[1]);
const exposure = new Map(rows.map((m) => [m[1], m[2]]));
if (table.length === 0) fail("the dispatch table parsed as empty");
else ok(`${table.length} rows in the dispatch table`);

// ── one against the other ────────────────────────────────────────────────────
const inShim = new Set(handlers);
const inTable = new Set(table);
const dupes = table.filter((n, i) => table.indexOf(n) !== i);
if (dupes.length) fail(`listed twice in the table: ${[...new Set(dupes)].join(", ")}`);
const missing = handlers.filter((n) => !inTable.has(n));
const extra = table.filter((n) => !inShim.has(n));
if (missing.length) fail(`in the shim but not dispatched over the socket: ${missing.join(", ")}`);
else ok("every shim command has a dispatch row");
if (extra.length) fail(`dispatched over the socket but not a shim command: ${extra.join(", ")}`);
else ok("every dispatch row is a shim command");

// ── the client-local set is the shim's own list ──────────────────────────────
// The commands the socket refuses as `client-local` are exactly the ones the
// api-boundary-check lets keep a body in the shim, plus the ones whose body
// opens something on the caller's machine. Pinned so a command cannot drift
// from "runs on the host" to "refused" (or back) without this line changing.
const CLIENT_LOCAL = ["copy_text", "open_editor", "open_terminal", "detect_editors", "open_docs_viewer", "set_zoom", "install_app_version"];
const tagged = table.filter((n) => exposure.get(n) === "ClientLocal").sort();
if (tagged.join() !== [...CLIENT_LOCAL].sort().join()) fail(`client-local rows are [${tagged.join(", ")}], expected [${CLIENT_LOCAL.join(", ")}]`);
else ok("the client-local set is the expected seven");

console.log(failed ? `\nserve-dispatch-check: ${failed} failure(s)` : "\nserve-dispatch-check: all good");
process.exit(failed ? 1 : 0);
