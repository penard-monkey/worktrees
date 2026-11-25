// A workspace's memory as a document source (app/src/doc/memory.ts,
// memorypath.ts), run as it is (vite's SSR loader) over a stubbed door.
//
//   node app/scripts/memory-docs-check.mjs      # from the repo root; exits non-zero on failure
//
// What it holds:
//   - links: a relative link resolves from the file it is in; nothing leaves
//     the memory, reaches a dot name, or is fetched from a network; what
//     `linkTo` writes, `memoryLink` reads back;
//   - the two refusals the save session reads (stale, gone) are the sentences
//     crates/worktrees-core/src/memory.rs sends;
//   - every `workspace_memory*` row is in serve's table, and each call sends
//     only arguments that row reads, and every one it must;
//   - the mock answers every one of them;
//   - the event: another workspace's is not this one's, and a path is the id;
//   - the source: one object a workspace; keep mine is said, an ordinary save
//     does not say it; a new file carries no mtime; a restore keeps what it
//     replaces; a kept text finds a free name; a picture already held is not
//     sent again; a dropped megabyte is encoded whole.
//
// Override, to show it red: MEMORYDOCS_ROOT=<another app/>, MEMORY_RS=<copy>.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const ROOT = process.env.MEMORYDOCS_ROOT ?? fileURLToPath(new URL("..", import.meta.url));
const REPO = fileURLToPath(new URL("../..", import.meta.url));
let failed = 0;
const check = (c, m) => { if (!c) failed++; console.log(`${c ? "ok" : "not ok"} — ${m}`); };
const eq = (a, b, m) => check(JSON.stringify(a) === JSON.stringify(b), `${m}${JSON.stringify(a) === JSON.stringify(b) ? "" : `: got ${JSON.stringify(a)}, want ${JSON.stringify(b)}`}`);
const settle = (p) => Promise.race([p, new Promise((_, no) => setTimeout(() => no(new Error("never settled")), 5000))]);

const calls = [];
let answer = {};
const listeners = new Map();
globalThis.__invoke = async (cmd, args) => {
  calls.push([cmd, args]);
  const a = answer[cmd];
  const r = typeof a === "function" ? a(args) : a;
  if (r instanceof Error) throw r.message;
  return r;
};
globalThis.__listen = async (name, fn) => { listeners.set(name, fn); return () => listeners.delete(name); };
const server = await createServer({
  root: ROOT, configFile: false, logLevel: "error", appType: "custom", server: { middlewareMode: true, hmr: false, ws: false }, optimizeDeps: { noDiscovery: true },
  plugins: [{
    name: "stub-ipc", enforce: "pre",
    resolveId: (id) => (/(^|\/)ipc$/.test(id) ? "\0ipc-stub" : null),
    load: (id) => (id === "\0ipc-stub" ? "export const invoke = (...a) => globalThis.__invoke(...a); export const listen = (...a) => globalThis.__listen(...a);" : null),
  }],
});
const P = await server.ssrLoadModule("/src/doc/memorypath.ts");
const { memoryDocs } = await server.ssrLoadModule("/src/doc/memory.ts");

// ── links ───────────────────────────────────────────────────────────────────
const L = P.memoryLink;
const file = (p, hash = null) => ({ kind: "file", path: p, hash });
eq(L("MEMORY.md", "topics/nas-layout.md"), file("topics/nas-layout.md"), "a link from the index");
eq(L("topics/nas-layout.md", "../notes/a.md#done"), file("notes/a.md", "done"), "a link from a topic, with a heading");
eq(L("topics/a.md", "b.md"), file("topics/b.md"), "a sibling");
eq(L("topics/a.md", "./deep/c%20d.md"), file("topics/deep/c d.md"), "an escaped space");
eq(L("a.md", "100%.md"), file("100%.md"), "an unescaped % is part of the name");
eq(L("a.md", "#here"), { kind: "anchor", hash: "here" }, "a heading of this file");
eq(L("a.md", "idea:01KV9AY8S6Q0B4M0N1P2R3S4T5"), { kind: "idea", id: "01KV9AY8S6Q0B4M0N1P2R3S4T5" }, "an idea");
eq(L("a.md", "https://x.y/z"), { kind: "url", url: "https://x.y/z" }, "a web page");
for (const bad of ["", "  ", "../x.md", "topics/../../x.md", "/etc/hosts", "//host/x.png", "file:///etc/hosts", "javascript:alert(1)", "data:image/png;base64,AA", "%2e%2e/x.md", "%2Fetc/hosts", ".git/config", "topics/.git/config", "a\\b.md", "a%00.md", "topics/..", "?x"]) eq(L("a.md", bad).kind, "none", `refused: ${JSON.stringify(bad)}`);
for (const [from, to] of [["MEMORY.md", "topics/a.md"], ["topics/a.md", "topics/b.md"], ["topics/a.md", "notes/c (d) é.md"], ["a/b/c.md", "a/d.md"], ["a/b.md", "MEMORY.md"], ["notes/x.md", "files/100%#1.png"]]) eq(L(from, P.linkTo(from, to)), file(to), `what is written from ${from} to ${to} reads back`);
eq([P.topicPath("NAS  layout!"), P.topicPath("", 3), P.topicPath("x.md")], ["topics/nas-layout.md", "topics/untitled-3.md", "topics/x.md"], "a kept text's name");

// ── the refusals, as core sends them ────────────────────────────────────────
const rs = fs.readFileSync(process.env.MEMORY_RS ?? path.join(REPO, "crates/worktrees-core/src/memory.rs"), "utf8");
const said = (n) => { const m = [...rs.matchAll(new RegExp(`^pub const ${n}: &str = "([^"]+)";`, "gm"))]; check(m.length === 1, `core declares ${n} once`); return m[0]?.[1] ?? ""; };
const stale = said("ERR_STALE"), gone = said("ERR_GONE");
check(P.isMemoryStale(`${stale}: MEMORY.md`) && !P.isMemoryGone(`${stale}: MEMORY.md`), "a stale save reads as stale");
check(P.isMemoryGone(`${gone}: a.md`) && !P.isMemoryStale(`${gone}: a.md`), "a missing file reads as gone");
check(/there is already a file at/.test(rs.slice(0, rs.search(/\n#\[cfg\(test\)\]\s*\nmod tests\b/))), "core's words for a taken name, which keepAsNew reads");

// ── the rows ────────────────────────────────────────────────────────────────
const disp = fs.readFileSync(path.join(REPO, "crates/worktrees-cli/src/serve/dispatch.rs"), "utf8");
const rows = new Map([...disp.matchAll(/^ {4}(workspace_memory\w*): HostConsole \[console, workspace\(id\)\] => (.*),$/gm)].map((m) => [m[1], [...m[2].matchAll(/a\.get\("(\w+)"\)/g)].map((x) => x[1])]));
check(rows.size === 9, `nine rows in serve's table (${rows.size})`);
const mock = fs.readFileSync(path.join(ROOT, "src/mock/install.ts"), "utf8");
const mm = fs.readFileSync(path.join(ROOT, "src/mock/memory.ts"), "utf8");
for (const r of rows.keys()) check(mock.includes(`case "${r}":`) && mm.includes(`case "${r}"`), `the mock answers ${r}`);
for (const ev of ["workspaces:memory", "workspaces:history"]) check(new RegExp(`\\("${ev}", Event::Console\\)`).test(fs.readFileSync(path.join(REPO, "crates/worktrees-cli/src/serve/scope.rs"), "utf8")), `${ev} is the console's`);
check(mm.includes('emit("workspaces:history"') && mm.includes("expectedRev") && rows.get("workspace_memory_write").includes("expected_rev"), "a save's rev and the history event, in the mock and the table");

// ── the source ──────────────────────────────────────────────────────────────
const D = memoryDocs("W1");
check(memoryDocs("W1") === D && memoryDocs("W2") !== D, "one object a workspace");
const put = (p, m = 7) => ({ path: p, file: p.split("/").pop(), title: "T", mtime_ms: m, size: 1 });
const run = async (f) => { calls.length = 0; return settle(f()); };
answer = {
  workspace_memory: { made: true }, workspace_memory_make: { made: true },
  workspace_memory_file: (a) => ({ ...put(a.path, a.commit ? 0 : 40), content: a.commit ? "old" : "new" }),
  workspace_memory_write: (a) => put(a.path, 41),
  workspace_memory_put: (a) => put(`files/${a.name}`),
  workspace_memory_image: (a) => (a.etag === "e1" ? { b64: "", unchanged: true, etag: "e1", mime: "image/png", size: 3, truncated: false } : { b64: "QUJD", unchanged: false, etag: "e1", mime: "image/png", size: 3, truncated: false }),
  workspace_memory_log: { changes: [
    { commit: "c2", at: 20, subject: "Edited: a.md", files: [{ path: "a.md", status: "D" }] },
    { commit: "c1", at: 10, subject: "fold: A", files: [{ path: "a.md", status: "M" }, { path: "b.md", status: "A" }] },
  ], more: false },
  workspace_memory_diff: (a) => ({ change: { commit: a.commit }, file: a.path ? { path: a.path, diff: { kind: "binary" } } : null }),
  workspace_memory_revert: { change: null, conflicts: ["a.md"] },
};
const all = [];
const go = async (f) => { const r = await run(f); all.push(...calls); return r; };
eq(await go(() => D.read("topics/a.md")), { id: "topics/a.md", title: "T", file: "a.md", mtime_ms: 40, content: "new" }, "a document is named by its path");
eq(await go(() => D.write("a.md", "x", 40, false)), { mtime_ms: 41, file: "a.md", title: "T" }, "a save answers what the session takes");
eq(calls[0][1], { id: "W1", path: "a.md", content: "x", expectedMtime: 40 }, "an ordinary save sends no keepHistory");
await go(() => D.write("a.md", "x", 40, true));
eq(calls[0][1].keepHistory, true, "keep mine says so");
await go(() => D.create("topics/n.md", "x"));
check(!("expectedMtime" in calls[0][1]), "a new file carries no mtime");
eq((await go(() => D.versions("a.md"))).map((v) => v.version), ["c1"], "a version is a change that left the file a text");
eq((await go(() => D.restore("a.md", "c1", undefined))).content, "old", "a restore answers the text it put back");
eq(calls.map(([c, a]) => [c, a.commit ?? null, a.expectedMtime ?? null, a.keepHistory ?? null]), [["workspace_memory_file", "c1", null, null], ["workspace_memory_file", null, null, null], ["workspace_memory_write", null, 40, true]], "and keeps what it replaced");
answer.workspace_memory_file = (a) => (a.commit ? { ...put(a.path, 0), content: "old" } : new Error(`${gone}: ${a.path}`));
await go(() => D.restore("a.md", "c1", undefined));
check(!("expectedMtime" in calls.at(-1)[1]) && !("keepHistory" in calls.at(-1)[1]), "a removed file's restore makes it again");
answer.workspace_memory_write = (a) => (a.path === "topics/t.md" ? new Error("there is already a file at topics/t.md") : put(a.path, 41));
eq((await go(() => D.keepAsNew("T", "x"))).id, "topics/t-2.md", "a kept text finds a free name");
answer.workspace_memory_write = () => new Error("disk full");
eq(await go(() => D.keepAsNew("T", "x")).then(() => "kept", (e) => e), "disk full", "and any other refusal is said, once");
check(calls.length === 1, "without trying again");
await go(() => D.view()); await go(() => D.make()); await go(() => D.log({ path: "a.md", before: "c1", n: 5 })); await go(() => D.change("c1")); await go(() => D.diff("c1", "a.md"));
eq(await go(() => D.revert("c1")), { change: null, conflicts: ["a.md"] }, "a conflict is answered, not thrown");
const big = new Uint8Array(1_500_000).fill(65);
await go(() => D.put("big.bin", big));
check(Buffer.from(calls[0][1].data, "base64").equals(Buffer.from(big)), "a dropped megabyte is encoded whole");
eq(await go(() => D.image("topics/a.md", "../files/r.png")), { src: "data:image/png;base64,QUJD", size: 3 }, "an image resolves from its file");
eq([await go(() => D.image("MEMORY.md", "files/r.png")), calls[0][1].etag], [{ src: "data:image/png;base64,QUJD", size: 3 }, "e1"], "one already held is not sent again");
eq([await run(() => D.image("a.md", "https://x.y/z.png")), calls.length], [{ note: "Remote image, not loaded." }, 0], "a remote image is never loaded");
eq([(await run(() => D.image("a.md", "../../x.png"))).note, calls.length], ["Outside this memory.", 0], "nor one outside");

// Every argument sent is one its row reads, and each row was called.
const sent = new Map();
for (const [cmd, a] of all) sent.set(cmd, new Set([...(sent.get(cmd) ?? []), ...Object.keys(a)]));
const snake = (k) => k.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
for (const [r, args] of rows) {
  const got = [...(sent.get(r) ?? [])].map(snake);
  check(sent.has(r), `${r} is called`);
  check(got.every((k) => args.includes(k)), `${r} is sent only what it reads (${got.join(", ")})`);
  // OWED: the row reads it and this source does not send it yet. Sent, its
  // entry here goes.
  const owed = r === "workspace_memory_write" ? ["expected_rev"] : [];
  check(args.every((k) => got.includes(k) !== owed.includes(k)), `and something sends each argument it reads (${args.join(", ")})`);
}

// ── the event ───────────────────────────────────────────────────────────────
const heard = [];
const off = D.watch((ev) => heard.push(ev));
await settle(new Promise((r) => setTimeout(r, 0)));
const push = (payload) => listeners.get("workspaces:memory")({ payload });
push({ id: "W2", changed: [{ path: "a.md", mtime_ms: 1 }], removed: [] });
eq(heard, [], "another workspace's change is not this one's");
push({ id: "W1", changed: [{ path: "a.md", mtime_ms: 9 }], removed: ["b.md"] });
eq(heard, [{ changed: [{ id: "a.md", mtime_ms: 9 }], removed: ["b.md"] }], "a change names the file as its session does");
off();
check(!listeners.has("workspaces:memory"), "and it stops listening");
await server.close();
console.log(failed ? `\n${failed} FAILED` : "\nall ok");
process.exit(failed ? 1 : 0);
