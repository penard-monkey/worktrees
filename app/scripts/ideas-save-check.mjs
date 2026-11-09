// The Ideas editor's save and conflict rules (app/companion/ideasave.ts), on
// a VIRTUAL clock with scripted rows.
//
//   node app/scripts/ideas-save-check.mjs      # from the repo root; exits non-zero on failure
//
// It loads the REAL module (as a `data:` module, which is why ideasave.ts may
// have no runtime import) and holds:
//   - autosave waits for the pause, and a flush saves at once;
//   - a text equal to the last saved one sends NO idea_write (typing and
//     undoing back, a flush with nothing new);
//   - one write at a time; an edit made while one is out is saved after it;
//   - a refused save keeps the person's text, stops saving, and Keep mine
//     writes it against the file's CURRENT mtime; Reload takes the file's;
//   - ideas:changed: an mtime any write of ours returned is ours, even one
//     a tick reported late between two saves; an event that arrives while a
//     write is out waits for its answer; any other mtime is an edit made
//     elsewhere, reloaded when clean and a conflict when not;
//   - a `removed` is confirmed with idea_read: found (an editor that saves
//     by rename) is a change, gone is `removed` — and nothing replaces the
//     editor's text either way while it is dirty.
//
// Override, so a mutation can be shown red without touching the tree:
//   IDEASAVE_SRC=<path to a copy of ideasave.ts>
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = process.env.IDEASAVE_SRC ?? here("../companion/ideasave.ts");
let failed = 0;
const check = (c, m) => { if (!c) failed++; console.log(`${c ? "ok" : "not ok"} — ${m}`); };

const src = fs.readFileSync(SRC, "utf8");
check(!/^\s*import\s(?!type\b)/m.test(src), "ideasave.ts has no runtime import (this check loads it as a data: module)");
const js = (await transformWithEsbuild(src, "ideasave.ts", { loader: "ts" })).code;
const M = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
const { Session, SAVE_PAUSE_MS } = M;

// ── the rig ─────────────────────────────────────────────────────────────────
function clock() {
  let t = 0, seq = 0;
  const timers = new Map();
  return {
    now: () => t,
    setTimeout: (fn, ms) => { const h = ++seq; timers.set(h, { at: t + ms, fn }); return h; },
    clearTimeout: (h) => { timers.delete(h); },
    pending: () => timers.size,
    async advance(ms) {
      const end = t + ms;
      for (;;) {
        const due = [...timers.entries()].filter(([, v]) => v.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        timers.delete(due[0]);
        t = due[1].at;
        due[1].fn();
        await settle();
      }
      t = end;
      await settle();
    },
  };
}
/** Let every queued promise reaction run. */
async function settle() { for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); }
/** Every await on the module races a deadline: a promise left pending would
 *  otherwise end the run with exit 13 and no failure printed (AGENTS.md). */
async function within(p, what) {
  let t;
  const r = await Promise.race([p.then(() => true), new Promise((res) => { t = setTimeout(() => res(false), 2000); })]);
  clearTimeout(t);
  if (!r) check(false, `${what} never settled`);
}

const STALE = "the idea changed on disk since you opened it — reload to see the latest";
const GONE = "no idea has the id 01JC";

/** A file on "disk", rows that write and read it, and a log of every call. */
function rig(initial = "---\nid: 01JC\n---\n\n# A\n", { holdWrites = false } = {}) {
  const disk = { content: initial, mtime: 1000, exists: true };
  const calls = [];
  const held = [];
  let editor = initial;
  const states = [];
  const replaced = [];
  const c = clock();
  const doWrite = (content, exp) => {
    if (!disk.exists) throw GONE;
    if (exp !== disk.mtime) throw STALE;
    if (content !== disk.content) { disk.content = content; disk.mtime += 10; }
    return disk.mtime;
  };
  const rows = {
    write(id, content, exp) {
      calls.push({ row: "idea_write", content, exp });
      if (!holdWrites) { try { return Promise.resolve(doWrite(content, exp)); } catch (e) { return Promise.reject(e); } }
      return new Promise((res, rej) => held.push(() => { try { res(doWrite(content, exp)); } catch (e) { rej(e); } }));
    },
    read() {
      calls.push({ row: "idea_read" });
      return disk.exists ? Promise.resolve({ content: disk.content, mtime_ms: disk.mtime }) : Promise.reject(GONE);
    },
  };
  const s = new Session("01JC", initial, disk.mtime, {
    rows, clock: c,
    text: () => editor,
    replace: (t) => { replaced.push(t); editor = t; },
    onState: (st) => states.push(st),
  });
  return {
    s, c, disk, calls, states, replaced,
    type(t) { editor = t; s.edited(); },
    get editor() { return editor; },
    writes: () => calls.filter((x) => x.row === "idea_write"),
    reads: () => calls.filter((x) => x.row === "idea_read"),
    release: async () => { held.shift()?.(); await settle(); },
    /** An edit made in another editor: the file's bytes and mtime move. */
    elsewhere(content) { disk.content = content; disk.mtime += 7; return { changed: [{ id: "01JC", mtime_ms: disk.mtime }], removed: [] }; },
    last: () => states[states.length - 1]?.kind,
  };
}

// ── autosave ────────────────────────────────────────────────────────────────
{
  const r = rig();
  r.type(r.editor + "x");
  await r.c.advance(SAVE_PAUSE_MS - 1);
  check(r.writes().length === 0, `no write before the pause (${SAVE_PAUSE_MS} ms)`);
  r.type(r.editor + "y");
  await r.c.advance(SAVE_PAUSE_MS - 1);
  check(r.writes().length === 0, "a keystroke restarts the pause");
  await r.c.advance(1);
  check(r.writes().length === 1 && r.writes()[0].content.endsWith("# A\nxy") && r.writes()[0].exp === 1000,
    "one write after the pause, of the editor's text, against the mtime it was opened at");
  check(r.last() === "clean" && r.disk.content === r.editor, "and the session is clean, the file holding the text");
  r.type(r.editor + "z");
  await r.s.flush();
  check(r.writes().length === 2 && r.writes()[1].exp === 1010, "a flush saves at once, against the mtime the last write returned");
}
{
  const r = rig();
  const orig = r.editor;
  r.type(orig + "x");
  r.type(orig);
  await r.c.advance(SAVE_PAUSE_MS * 3);
  await r.s.flush();
  check(r.writes().length === 0, "typed and undone back to the saved text: no idea_write, on the pause or on a flush");
  check(r.last() === "clean", "and the session reads clean");
  await r.s.flush();
  check(r.writes().length === 0 && r.c.pending() === 0, "a flush with nothing new sends nothing and leaves no timer");
}

// ── one write at a time ─────────────────────────────────────────────────────
{
  const r = rig(undefined, { holdWrites: true });
  r.type(r.editor + "1");
  await r.c.advance(SAVE_PAUSE_MS);
  check(r.writes().length === 1 && r.last() === "saving", "the first write is out");
  r.type(r.editor + "2");
  const f = r.s.flush();
  await settle();
  check(r.writes().length === 1, "a flush while a write is out does not start a second one");
  await r.release();
  check(r.writes().length === 2 && r.writes()[1].content.endsWith("12") && r.writes()[1].exp === 1010,
    "the edit made during it is saved after it, against the mtime it returned");
  await r.release();
  await within(f, "the flush");
  check(r.last() === "clean" && r.disk.content === r.editor, "both edits are on disk");
}

// ── a refused save ──────────────────────────────────────────────────────────
{
  const r = rig();
  r.disk.content = "---\nid: 01JC\n---\n\n# Theirs\n"; r.disk.mtime = 5000; // moved under us, no event yet
  const mine = r.editor + "mine";
  r.type(mine);
  await r.c.advance(SAVE_PAUSE_MS);
  check(r.last() === "conflict" && r.states.at(-1).why === "refused", "a write refused for a stale mtime is a conflict");
  check(r.editor === mine && r.replaced.length === 0, "and the person's text is still in the editor, untouched");
  r.type(mine + "!");
  await r.c.advance(SAVE_PAUSE_MS * 2);
  await r.s.flush();
  check(r.writes().length === 1, "while in conflict nothing saves, on the pause or on a flush");
  await r.s.keepMine();
  const w = r.writes()[1];
  check(w && w.exp === 5000 && w.content === mine + "!" && r.disk.content === mine + "!", "Keep mine writes the editor's text against the file's current mtime");
  check(r.last() === "clean", "and the session is clean again");
}
{
  const r = rig();
  r.disk.content = "---\nid: 01JC\n---\n\n# Theirs\n"; r.disk.mtime = 5000;
  r.type(r.editor + "mine");
  await r.c.advance(SAVE_PAUSE_MS);
  await r.s.reload();
  check(r.editor === r.disk.content && r.last() === "clean", "Reload takes the file's text");
  r.type(r.editor + "more");
  await r.c.advance(SAVE_PAUSE_MS);
  check(r.writes().at(-1).exp === 5000 && r.last() === "clean", "and the next save is against the file's mtime");
}

// ── ideas:changed: ours, and elsewhere ──────────────────────────────────────
{
  const r = rig();
  r.type(r.editor + "a");
  await r.s.flush();
  const m1 = r.disk.mtime;
  r.type(r.editor + "b");
  await r.s.flush();
  await r.s.external({ changed: [{ id: "01JC", mtime_ms: m1 }], removed: [] });
  check(r.reads().length === 0 && r.replaced.length === 0 && r.last() === "clean",
    "an event carrying the mtime of an EARLIER write of ours (a tick between two saves) is ours: nothing is read or replaced");
  await r.s.external({ changed: [{ id: "01JC", mtime_ms: r.disk.mtime }], removed: [] });
  check(r.reads().length === 0 && r.replaced.length === 0, "the last write's mtime is ours too");
  await r.s.external({ changed: [{ id: "01OTHER", mtime_ms: 1 }], removed: ["01GONE"] });
  check(r.reads().length === 0 && r.last() === "clean", "another idea's change is not this one's");
}
{
  const r = rig(undefined, { holdWrites: true });
  r.type(r.editor + "a");
  await r.c.advance(SAVE_PAUSE_MS);
  // The poll saw the write land before its answer reached us.
  const ev = { changed: [{ id: "01JC", mtime_ms: 1010 }], removed: [] };
  const done = r.s.external(ev);
  await settle();
  check(r.reads().length === 0 && r.last() === "saving", "an event during a write waits for the write's answer");
  await r.release();
  await within(done, "the deferred event");
  check(r.reads().length === 0 && r.replaced.length === 0 && r.last() === "clean", "and the mtime it carried was that write's: ours");
}
{
  const r = rig();
  const ev = r.elsewhere("---\nid: 01JC\n---\n\n# Edited elsewhere\n");
  await r.s.external(ev);
  check(r.reads().length === 1 && r.editor === r.disk.content && r.last() === "clean", "an edit elsewhere with the editor clean: re-read and reloaded");
  r.type(r.editor + "x");
  await r.c.advance(SAVE_PAUSE_MS);
  check(r.writes().at(-1)?.exp === 1007, "the next save is against the reloaded mtime");
}
{
  const r = rig();
  const mine = r.editor + "mine";
  r.type(mine);
  const ev = r.elsewhere("---\nid: 01JC\n---\n\n# Edited elsewhere\n");
  await r.s.external(ev);
  check(r.last() === "conflict" && r.states.at(-1).why === "elsewhere", "an edit elsewhere while the editor is dirty is a conflict");
  check(r.editor === mine && r.replaced.length === 0, "and the person's text is kept");
  await r.c.advance(SAVE_PAUSE_MS * 2);
  check(r.writes().length === 0, "and the pending autosave did not write over the other edit");
}

// ── removed ─────────────────────────────────────────────────────────────────
{
  const r = rig();
  r.disk.content = "---\nid: 01JC\n---\n\n# Saved by rename\n"; r.disk.mtime = 7000;
  await r.s.external({ changed: [], removed: ["01JC"] });
  check(r.reads().length === 1 && r.last() !== "removed" && r.editor === r.disk.content,
    "a `removed` whose idea_read finds the file (saved by rename): a change, reloaded");
}
{
  const r = rig();
  await r.s.external({ changed: [], removed: ["01JC"] });
  check(r.reads().length === 1 && r.states.length === 0 && r.replaced.length === 0, "a `removed` whose read finds the same mtime: nothing changed");
}
{
  const r = rig();
  const mine = r.editor + "mine";
  r.type(mine);
  r.disk.exists = false;
  await r.s.external({ changed: [], removed: ["01JC"] });
  check(r.reads().length === 1 && r.last() === "removed", "a `removed` confirmed by idea_read is `removed`");
  check(r.editor === mine && r.replaced.length === 0, "and the person's unsaved text stays in the editor");
  await r.c.advance(SAVE_PAUSE_MS * 2);
  await r.s.flush();
  check(r.writes().length === 0, "and nothing tries to save over a removed idea");
}

// ── close ───────────────────────────────────────────────────────────────────
{
  const r = rig();
  r.type(r.editor + "x");
  r.s.close();
  await r.c.advance(SAVE_PAUSE_MS * 2);
  check(r.writes().length === 0 && r.c.pending() === 0, "a closed session's timer never fires (the window flushes before it closes one)");
}

console.log(failed ? `\nideas-save-check: ${failed} failure(s)` : "\nideas-save-check: all good");
process.exit(failed ? 1 : 0);
