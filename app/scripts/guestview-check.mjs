// guestview-check — the guest's views (group chat §7) hold no route to an
// agent, and their rules do what §3, §6 and §7 say.
//
//   node app/scripts/guestview-check.mjs        # from the repo root; exits non-zero on failure
//
// 1. THE GRAPH. From the guest's views (GuestSession.tsx, GuestNav.tsx) every
//    RUNTIME import is walked (`import type` is erased at build and skipped).
//    No module in it may be the engine door (ipc.ts), the socket client
//    (ipcws.ts), the chat (chat.ts, ChatPane.tsx) or the guest boot, and none
//    may name `agent_send`, `agent_answer*`, `agent_interrupt` or
//    `agent_command` as a string: a guest view reaches the owner's host only
//    through guestdoor.ts, which sends the guest rows alone (transport-check).
//    And the session view draws no composer, Stop or prompt card.
// 2. THE RULES, run as the real guestview.ts: a poll's page replaces the
//    turns it covers and nothing older; an older page is prepended; a turn's
//    pieces keep their steps' order; an ended or unanswered state reads as
//    §3.3 and D8 say; a re-read toasts only what is new; the badge counts the
//    shares not seen as they now are; a row goes under the local project of
//    its name, else Shared with me.
// 3. THE MIRRORS: a request note's limit is shares.rs's, a page's size within
//    history.rs's, the poll the chat view's.
//
// Overrides, so a mutation can be shown red without touching the tree:
//   GUESTVIEW_SESSION, GUESTVIEW_NAV, GUESTVIEW_RULES
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = {
  dir: here("../src"),
  session: process.env.GUESTVIEW_SESSION ?? here("../src/GuestSession.tsx"),
  nav: process.env.GUESTVIEW_NAV ?? here("../src/GuestNav.tsx"),
  rules: process.env.GUESTVIEW_RULES ?? here("../src/guestview.ts"),
  pane: here("../src/ChatPane.tsx"),
  shares: here("../../crates/worktrees-cli/src/serve/shares.rs"),
  history: here("../../crates/worktrees-cli/src/serve/history.rs"),
};
const read = (p) => fs.readFileSync(p, "utf8");
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const expect = (what, got, want) => check(same(got, want), `${what}${same(got, want) ? "" : `: want ${JSON.stringify(want)}, got ${JSON.stringify(got)}`}`);

// ── 1. the graph ─────────────────────────────────────────────────────────────
const FORBIDDEN_MODULES = ["ipc.ts", "ipcws.ts", "chat.ts", "ChatPane.tsx", "guestboot.ts", "ipcremote.ts"];
const FORBIDDEN_ROWS = /["'`](agent_send|agent_answer\w*|agent_interrupt|agent_command|idea_send|idea_answer\w*|idea_interrupt|workspace_send|workspace_answer\w*|workspace_interrupt)["'`]/;

/** The runtime relative imports of `src`: every `import … from "./x"` that is
 *  not `import type`, and every dynamic `import("./x")`. CSS is not code. */
function runtimeImports(src) {
  const out = [];
  for (const m of src.matchAll(/^\s*import\s+(type\s+)?([^"';]*?)\s*from\s*["'](\.[^"']+)["']/gm)) if (!m[1]) out.push(m[3]);
  for (const m of src.matchAll(/^\s*import\s+["'](\.[^"']+)["']/gm)) out.push(m[1]);
  for (const m of src.matchAll(/import\(\s*["'](\.[^"']+)["']\s*\)/g)) out.push(m[1]);
  return out.filter((p) => !p.endsWith(".css") && !p.endsWith(".png"));
}
function resolve(from, spec) {
  const base = path.resolve(path.dirname(from), spec);
  for (const c of [base, `${base}.ts`, `${base}.tsx`, path.join(base, "index.ts")]) if (fs.existsSync(c) && fs.statSync(c).isFile()) return c;
  return null;
}
function graph(roots) {
  const seen = new Map();
  const walk = (f, via) => {
    if (seen.has(f)) return;
    seen.set(f, via);
    for (const spec of runtimeImports(read(f))) {
      const r = resolve(f, spec);
      if (r) walk(r, f);
    }
  };
  for (const r of roots) walk(r, null);
  return seen;
}
{
  const g = graph([SRC.session, SRC.nav]);
  const names = [...g.keys()].map((f) => path.relative(SRC.dir, f));
  check(names.includes("guestdoor.ts") && names.includes("guest.ts") && names.includes("ThreadPanel.tsx"), `the walk reaches the guest door, the store and the thread (${names.length} modules)`);
  const bad = names.filter((n) => FORBIDDEN_MODULES.includes(path.basename(n)));
  const chain = (n) => { const out = [n]; let v = g.get(path.join(SRC.dir, n)); while (v) { out.push(path.relative(SRC.dir, v)); v = g.get(v); } return out.reverse().join(" → "); };
  check(bad.length === 0, `no guest view imports the engine door, the socket client, the chat or the boot${bad.length ? `: ${bad.map(chain).join("; ")}` : ""}`);
  // Code, not prose: a comment may say why a row is out of reach.
  const code = (f) => read(f).replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
  const naming = [...g.keys()].filter((f) => FORBIDDEN_ROWS.test(code(f))).map((f) => `${path.relative(SRC.dir, f)} (${code(f).match(FORBIDDEN_ROWS)[1]})`);
  check(naming.length === 0, `no module in the guest views' graph names a row that drives an agent${naming.length ? `: ${naming.join(", ")}` : ""}`);
  const s = read(SRC.session);
  const drawn = ["<Composer", "stopTurn", "<PromptCard", "SlashMenu", "sendDraft", "sendAnswer"].filter((w) => s.includes(w));
  check(drawn.length === 0, `the session view draws no composer, Stop, prompt card or slash menu${drawn.length ? `: ${drawn.join(", ")}` : ""}`);
  check(/from "\.\/chat"/.test(s) ? /^import type [^\n]*from "\.\/chat";$/m.test(s) && !/^import \{[^\n]*from "\.\/chat";$/m.test(s) : true, "the session view takes only TYPES from chat.ts");
}

// ── 2. the rules ─────────────────────────────────────────────────────────────
const rulesSrc = read(SRC.rules);
if (/^import (?!type )/m.test(rulesSrc)) fail("guestview.ts gained a runtime import: it must load as a data: module");
const js = (await transformWithEsbuild(rulesSrc, "guestview.ts", { loader: "ts", format: "esm" })).code;
const R = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

const T = (seq, user = `u${seq}`) => ({ seq, at: 1000 + seq, user, reply: "", tools: [], truncated: false, steps: [{ kind: "text", at: 1001 + seq, text: `r${seq}` }] });
const seqs = (ts) => ts.map((t) => `${t.seq}:${t.user}`);
{
  const held = [T(0), T(1), T(2), T(3)];
  expect("a poll's page replaces the turns it covers and keeps the older ones", seqs(R.mergeNewest(held, [T(4), T(3, "u3'"), T(2, "u2'")])), ["0:u0", "1:u1", "2:u2'", "3:u3'", "4:u4"]);
  expect("an empty page changes nothing", seqs(R.mergeNewest(held, [])), seqs(held));
  expect("the first page is the newest, oldest first", seqs(R.mergeNewest([], [T(9), T(8)])), ["8:u8", "9:u9"]);
  check(R.leavesGap([T(0), T(1)], [T(9), T(8)]) && !R.leavesGap([T(0), T(1)], [T(3), T(2)]) && !R.leavesGap([], [T(9)]), "a poll whose page does not reach what is held is a gap, and stands alone");
  expect("an older page goes before, never over, what is held", seqs(R.mergeOlder([T(5), T(6)], [T(5, "dup"), T(4), T(3)])), ["3:u3", "4:u4", "5:u5", "6:u6"]);
  // Two turns can share one index seq (history_turns of one span): both stay.
  const twin = [{ ...T(7), user: "a" }, { ...T(7), user: "b" }];
  expect("turns sharing a seq are kept together", seqs(R.mergeNewest([T(6)], [...twin].reverse())), ["6:u6", "7:a", "7:b"]);
}
{
  const t = { seq: 1, at: 10, user: "go", reply: "", tools: [], truncated: false, steps: [
    { kind: "text", at: 11, text: "reading" },
    { kind: "tool", at: 12, name: "Read", call_id: "c1", summary: "a.ts" },
    { kind: "tool", at: 13, name: "Bash", call_id: "c2", summary: "ls" },
    { kind: "user", at: 14, text: "also this" },
    { kind: "tool", at: 15, name: "Edit", call_id: "c3" },
    { kind: "permission", at: 16, tool: "Bash", allowed: true, answered_at: 17 },
    { kind: "question", at: 18, questions: [{ header: "h", question: "Which?", multi: false, options: [] }] },
  ] };
  const p = R.piecesOf(t);
  expect("a turn's pieces keep its steps' order, tools under the block they followed", p.map((x) => x.k === "agent" ? `agent:${x.text}:${x.calls.map((c) => c.name).join("+")}` : x.k === "user" ? `user:${x.text}:${x.mid}` : `${x.k}:${x.st?.kind ?? ""}`),
    ["user:go:false", "agent:reading:Read+Bash", "user:also this:true", "agent::Edit", "asked:permission", "asked:question"]);
  expect("a notice turn is its notice", R.piecesOf({ seq: 2, at: 1, user: "x", reply: "", tools: [], truncated: false, notice: { kind: "interrupt" }, steps: [] }).map((x) => x.k), ["notice"]);
  expect("an older host's turn is its reply and its tools", R.piecesOf({ seq: 3, at: 1, user: "q", reply: "a", tools: ["Bash"], truncated: false }).map((x) => x.k === "agent" ? `${x.text}|${x.calls.length}` : x.k), ["user", "a|1"]);
}
{
  const share = (state, b) => ({ id: "s", project: "p", project_id: "x", slug: "s", title: null, state, binding: b, ended: [], created_at: 1 });
  const active = { harness: "claude", state: "active", bound_at: 1, ended_at: null };
  const ended = { ...active, state: "ended", ended_at: 2 };
  const up = { kind: "up" };
  // A host that never set its name (v0.52.0's three-VM run): null, or that
  // release's placeholder "owner", is not a name; the host's own is.
  expect("the owner's name, as the host answers it", R.ownerOf("Cy", "Cy", "cy-debian"), "Cy");
  expect("a host that never set one is called by its own name", R.ownerOf(null, null, "cy-debian"), "cy-debian");
  expect("a v0.52.0 host's placeholder \"owner\" is not a name", R.ownerOf("owner", "owner", "cy-debian"), "cy-debian");
  expect("guest_places' name when shares_list has none", R.ownerOf(null, "Cy", "cy-debian"), "Cy");
  expect("an owner\u2019s blank name is not one either", R.ownerOf(" ", undefined, "cy-debian"), "cy-debian");
  expect("an allowed, active share is live", R.standing(share("allowed", active), active, up, "David"), { live: true, words: null });
  expect("an ended binding is frozen, never live", R.standing(share("allowed", ended), ended, up, "David").live, false);
  check(/Ended/.test(R.standing(share("allowed", ended), ended, up, "David").words), "an ended binding says Ended");
  expect("an asked share waits for the owner", R.standing(share("asked", active), active, up, "David").words, "Waiting for David to let you in");
  expect("offline says the owner's host is offline, before anything else", R.standing(share("asked", active), active, { kind: "offline", why: "x", retryAt: null }, "David"), { live: false, words: "David's host is offline" });
  // A removed guest is told so, not "offline" (the v0.52.0 three-VM run: Bret's app said offline forever).
  expect("a guest the owner removed is told so", R.standing(share("allowed", active), active, { kind: "removed", why: "x", by: "revoked" }, "David"), { live: false, words: "David removed this device" });
  expect("a forgotten device reads the same", R.standing(share("allowed", active), active, { kind: "removed", why: "x", by: "removed" }, "David").words, "David removed this device");
  expect("a disabled device is told it was turned off", R.standing(share("allowed", active), active, { kind: "removed", why: "x", by: "disabled" }, "David").words, "David's host turned this device off");
  expect("a refused key says the host no longer accepts it", R.standing(share("allowed", active), active, { kind: "removed", why: "x", by: "refused" }, "David").words, "David's host no longer accepts this device");
}
{
  const s = (id, state, bound = 1, slug = id) => ({ id, project: "worktrees", project_id: "x", slug, title: null, state, binding: { harness: "claude", state: "active", bound_at: bound, ended_at: null }, ended: [], created_at: 1 });
  expect("a connect toasts nothing (the badge says it)", R.newsOf("David", null, [s("a", "allowed")]), []);
  expect("a new share, a let-in and a share-again toast, in §6's words", R.newsOf("David", [s("a", "asked"), s("b", "allowed")], [s("a", "allowed"), s("b", "allowed", 9), s("c", "invited", 1, "feat-x")]),
    ["David let you in on a", "David shared b again", "David shared feat-x with you"]);
  const seen = { a: R.shareMark(s("a", "allowed")) };
  expect("the badge counts shares never opened or moved since", R.unseenOf([s("a", "allowed"), s("b", "invited")], seen), ["b"]);
  expect("…and one whose standing moved", R.unseenOf([s("a", "allowed", 5)], seen), ["a"]);
}
{
  const lists = { shares: [{ project: "worktrees", id: "1" }, { project: "garden", id: "2" }], projects: [{ name: "worktrees", id: "w", places: [] }, { name: "garden", id: "g", places: [] }] };
  expect("a share goes under the local project of its name", R.placed(lists, "worktrees", ["worktrees"]).shares.map((x) => x.id), ["1"]);
  expect("…and everything matching no local project under Shared with me", R.placed(lists, null, ["worktrees"]).projects.map((x) => x.name), ["garden"]);
  // Offline (the 2026-11-14 rehearsal): a host listed under its local
  // project AND as an empty *Shared with me › David's Places · offline*.
  // Its empty status group goes where its rows went: under the local project
  // it matched, by the lists it holds or, before any read this run, by the
  // project names it matched last time (`known`).
  const held = { ...lists, projects: [lists.projects[0]], shares: [lists.shares[0]] };
  check(R.placed(held, "worktrees", ["worktrees"]).status === true, "an offline host that matched a local project keeps its status there");
  check(R.placed(held, null, ["worktrees"]).status === false, "…and not as an empty group under Shared with me too");
  const cold = { shares: null, projects: null, known: ["worktrees"] };
  check(R.placed(cold, "worktrees", ["worktrees"]).status === true && R.placed(cold, null, ["worktrees"]).status === false, "a host never read this run stays where its known names put it");
  const never = { shares: null, projects: null };
  check(R.placed(never, null, ["worktrees"]).status === true && R.placed(never, "worktrees", ["worktrees"]).status === false, "a host with no names at all is under Shared with me");
  check(R.placed(lists, null, ["worktrees"]).status === false, "a host that matched a local project has no status group of its own under Shared with me");
  check(R.placed(lists, null, ["elsewhere"]).status === true, "…while one that matched none has it there");
  expect("an invited row says opening asks to join", R.shareWords({ state: "invited", binding: null }, "David").words, "Open to ask to join");
}

// ── 3. the mirrors ───────────────────────────────────────────────────────────
{
  const noteMax = Number(/pub const NOTE_MAX: usize = (\d+);/.exec(read(SRC.shares))?.[1]);
  const navMax = Number(/export const NOTE_MAX = (\d+);/.exec(read(SRC.nav))?.[1]);
  check(noteMax > 0 && noteMax === navMax, `a request note's limit is shares.rs's NOTE_MAX (${navMax} against ${noteMax})`);
  const pageMax = Number(/pub const PAGE_TURNS_MAX: usize = (\d+);/.exec(read(SRC.history))?.[1]);
  const page = Number(/export const SHARED_PAGE = (\d+);/.exec(read(SRC.session))?.[1]);
  check(page > 0 && page <= pageMax, `a page asks for at most history.rs's PAGE_TURNS_MAX (${page} of ${pageMax})`);
  const busy = /const BUSY_POLL_MS = ([\d_]+);/.exec(read(SRC.pane))?.[1];
  const poll = /export const SHARED_POLL_MS = ([\d_]+);/.exec(read(SRC.session))?.[1];
  check(busy && busy === poll, `the shared view polls at the chat view's BUSY_POLL_MS (§3.2: ${poll} against ${busy})`);
}

console.log(failed ? `\nguestview-check: ${failed} failure(s)` : "\nguestview-check: all good");
process.exit(failed ? 1 : 0);
