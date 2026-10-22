// serve's dispatch table and the shim never both run a command.
//
//   node app/scripts/serve-dispatch-check.mjs      # exits non-zero on failure
//
// `worktrees serve` answers the same `{cmd, args}` the frontend's `invoke`
// sends, out of one table in `crates/worktrees-cli/src/serve/dispatch.rs` that
// pairs each command with its exposure. The desktop app is serve's CLIENT (one
// engine per host): its shim registers only serve's `ClientLocal` rows and its
// own two commands, and everything else goes over the socket. So the rule this
// check holds is the inverse of the one it held while the app was an engine:
// the shim's `generate_handler!` may contain NO `Engine`, `HostConsole`,
// `Loopback` or `Stream` row — one that did would be a second engine for that command, with
// its own registries, invisible to serve. The client-local set is pinned by
// name so a command cannot drift between "runs on the host" and "refused"
// without this line changing. (Which names the PAGE routes where, and that
// every `invoke("…")` it makes is a row or a shim command, is
// engine-route-check.mjs.)
//
// The wire those rows are reached over (`crates/worktrees-cli/src/serve/wire.rs`
// is the source of truth; this is the summary a client author needs):
//
//   handshake v1, before anything else (policy.rs; the token is the HMAC key
//   and is never sent; cn, sn and both proofs are 32 bytes as lowercase hex):
//            → {hello: cn}   ← {challenge: sn}
//            → {auth: HMAC(token, "worktrees-serve/v1/client\n" + cn + "\n" + sn)}
//            ← {ok: true, version, proof: HMAC(token, "worktrees-serve/v1/server\n" + cn + "\n" + sn)}
//            any other frame, a wrong proof, or 3 s for the whole exchange:
//            the socket closes with nothing written. Verify `proof` before
//            believing `version` or sending anything else. Before the proof
//            verifies a peer may send 512 bytes in all; after it, one message
//            (or frame) may be 16 MiB, and a larger one ends the connection.
//   request  {id, cmd, args}          id echoed verbatim; args as `invoke` sends
//   reply    {id, ok: <value>}        or {id, err: <value>} — err is what the
//                                     Tauri command's Err serializes to
//   channel  {channel, seq, data}     a ByteSink (data: base64) or a typed
//                                     progress value; seq counts from 1 per
//                                     channel; channel is the request's
//                                     `__CHANNEL__:<id>`
//   end      {channel, seq, end: true} the LAST frame on a channel: no data,
//                                     seq one past the last data frame. Sent
//                                     when the engine lets the sink go — a
//                                     detach, term_close, the shell exiting,
//                                     the operation behind a progress callback
//                                     returning. Drop the handler on it. An
//                                     err reply to the request that named the
//                                     channel also finishes it (an end frame
//                                     precedes it only if a sink was made).
//   event    {event, payload}         Host::emit, to every authenticated socket
//
// Every frame for one connection goes through ONE queue, so order holds across
// kinds: a channel's end frame cannot overtake its data, a reply cannot
// overtake the progress it finished. Requests to ONE stream (a `Stream` row
// naming an existing attach or dock shell) run on a serial lane per stream, so
// keystrokes reach the pty in the order they were sent; everything else runs
// concurrently, as Tauri's async commands do.
//
// Parsed from the sources rather than from a build: the table is a macro
// invocation and the handler list is a macro argument, and neither is a Rust
// item a test could enumerate.
import fs from "node:fs";
import { fileURLToPath } from "node:url";

// DISPATCH_SHIM / DISPATCH_TABLE point the check at a mutated copy.
const SHIM = process.env.DISPATCH_SHIM ?? fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url));
const TABLE = process.env.DISPATCH_TABLE ?? fileURLToPath(new URL("../../crates/worktrees-cli/src/serve/dispatch.rs", import.meta.url));

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
// Rows are `    name: Exposure [capability, object, …] => …` at exactly four
// spaces of indent inside the `commands! { cx, a;` invocation (the bracket is
// Phase 2's capability and object columns, absent on a bare `HostConsole` or
// `ClientLocal` row); bodies are indented deeper, so a line inside one cannot
// be mistaken for a row.
const src = fs.readFileSync(TABLE, "utf8");
const open = src.indexOf("commands! { cx, a;");
const close = open < 0 ? -1 : src.indexOf("\n}\n", open);
if (open < 0 || close < 0) fail("the `commands! { cx, a; … }` block is not where this check looks");
const block = open < 0 ? "" : src.slice(open, close);
const rows = [...block.matchAll(/^ {4}([a-z][a-z0-9_]*): (Engine|ClientLocal|HostConsole|Loopback|Stream)\b[^\n]*?=>/gm)];
const table = rows.map((m) => m[1]);
const exposure = new Map(rows.map((m) => [m[1], m[2]]));
if (table.length === 0) fail("the dispatch table parsed as empty");
else ok(`${table.length} rows in the dispatch table`);

// ── one against the other ────────────────────────────────────────────────────
const dupes = table.filter((n, i) => table.indexOf(n) !== i);
if (dupes.length) fail(`listed twice in the table: ${[...new Set(dupes)].join(", ")}`);
else ok("no command is listed twice");
const engineSide = handlers.filter((n) => exposure.has(n) && exposure.get(n) !== "ClientLocal");
if (engineSide.length) fail(`the shim registers engine commands serve also runs: ${engineSide.map((n) => `${n} (${exposure.get(n)})`).join(", ")} — a second engine for each`);
else ok("the shim registers no Engine, HostConsole, Loopback or Stream row — every engine command is serve's alone");
const localRows = table.filter((n) => exposure.get(n) === "ClientLocal");
const unregistered = localRows.filter((n) => !handlers.includes(n));
if (unregistered.length) fail(`ClientLocal rows the shim does not register: ${unregistered.join(", ")} — serve refuses them and nothing else runs them`);
else ok("every ClientLocal row is registered by the shim");

// ── the client-local set, by name ────────────────────────────────────────────
// Pinned so a command cannot drift from "runs on the host" to "refused" (or
// back) without this line changing. `open_docs_viewer` left it on 2026-10-20:
// its body registers a place with the HOST's documentation server, which the
// host poll refreshes, and the browser is opened by the page's own `openUrl`.
const CLIENT_LOCAL = ["copy_text", "open_editor", "open_terminal", "detect_editors", "set_zoom", "install_app_version"];
const tagged = table.filter((n) => exposure.get(n) === "ClientLocal").sort();
if (tagged.join() !== [...CLIENT_LOCAL].sort().join()) fail(`client-local rows are [${tagged.join(", ")}], expected [${CLIENT_LOCAL.join(", ")}]`);
else ok("the client-local set is the expected six");

console.log(failed ? `\nserve-dispatch-check: ${failed} failure(s)` : "\nserve-dispatch-check: all good");
process.exit(failed ? 1 : 0);
