// Browser gate for a memory as a document source: the REAL document pane on
// `memoryDocs`, against the mock, in Chromium AND WebKit.
//
//   VITE_MOCK=1 vite --port 1478 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     MEMORY_MOCK_URL=http://localhost:1478 node app/scripts/memory-browser.mjs
// MEMORY_SCREENSHOTS=<dir> saves the bench per engine.
//
// What it holds: the seeded memory lists and opens; typing saves by itself,
// with the mtime it read, and its own event reloads nothing; an agent's edit
// reaches a clean editor and meets unsaved text as a conflict, where Keep mine
// commits what it overwrites; a removed file says so and keeps the text; a
// relative image draws; a change shows one file's diff, a changed image both
// sides; Undo is a commit and its row then says Redo; a version restores.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.MEMORY_MOCK_URL ?? "http://localhost:1478";
const SHOTS = process.env.MEMORY_SCREENSHOTS;
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

for (const [name, engine] of [["chromium", chromium], ["webkit", webkit]]) {
  const browser = await engine.launch();
  const page = await browser.newPage({ viewport: { width: 1400, height: 860 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${BASE}/?workspaces=1&memtick=150`);
  const until = (fn, arg, ms = 6000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
  await until(() => !!window.__mock);
  const id = await page.evaluate(async () => {
    const { invoke } = await import("/src/ipc.ts");
    const id = (await invoke("workspaces_list")).workspaces[0].id;
    (await import("/scripts/memory-pane-harness.tsx")).mount(id);
    return id;
  });
  const say = (m) => `${name}: ${m}`;
  const ed = ".ix-doc .cm-content";
  ok(await until(() => document.querySelectorAll('[data-testid="mem-list"] [data-path]').length === 5), say("five files: the index, two notes, a topic, an image"));
  ok(await until((s) => document.querySelector(s)?.textContent?.includes("What this workspace is for"), ed), say("the index opens in the editor"));
  ok(await page.locator('[data-testid="mem-log"] [data-commit]').count() === 5, say("five changes"));
  const kinds = await page.locator('[data-testid="mem-log"] [data-kind]').evaluateAll((e) => e.map((x) => x.dataset.kind));
  ok(kinds.join() === "edit,fold,undo,fold,made", say(`newest first (${kinds})`));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/memory-index.${name}.png` });

  // Typing saves by itself.
  await page.click(ed);
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type("\nTYPED-BY-ME\n");
  ok(await until(() => window.__mock.memoryWrites().some((w) => w.content.includes("TYPED-BY-ME"))), say("it saved by itself"));
  const w = await page.evaluate(() => window.__mock.memoryWrites().at(-1));
  ok(typeof w.expected_mtime === "number" && !w.keep_history, say("against the mtime it read, keeping no copy"));
  await page.waitForTimeout(500); // the poll's word on our own save
  ok(await page.evaluate((s) => document.querySelector(s).textContent.includes("TYPED-BY-ME"), ed) && !(await page.locator('[data-testid="ideas-conflict"]').count()), say("its own event is not an edit made elsewhere"));

  // An agent's edit reaches a clean editor.
  await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# Memory: agent\n\nAGENT-ONE\n"), id);
  ok(await until((s) => document.querySelector(s)?.textContent?.includes("AGENT-ONE"), ed), say("an agent's edit reaches a clean editor"));
  // …and meets unsaved text as a conflict.
  await page.click(ed);
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type("MINE");
  await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# Memory: agent\n\nAGENT-TWO\n"), id);
  const keep = page.getByRole("button", { name: /keep mine/i });
  ok(await keep.waitFor({ timeout: 6000 }).then(() => true, () => false), say("over unsaved text it is a conflict"));
  ok(await page.evaluate((s) => document.querySelector(s).textContent.includes("MINE"), ed), say("the person's text stays"));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/memory-conflict.${name}.png` });
  await keep.click();
  ok(await until(() => window.__mock.memoryWrites().at(-1).keep_history === true), say("Keep mine says so"));
  ok(await until(() => [...document.querySelectorAll('[data-testid="mem-log"] [data-commit]')].some((e) => e.textContent.includes("Before keeping your text"))), say("and what it wrote over is in the history"));

  // Images, a diff, an undo.
  await page.click('[data-path="topics/nas-layout.md"]');
  ok(await until(() => { const i = document.querySelector('[data-testid="mem-pics"] img'); return i?.complete && i.naturalWidth > 0; }), say("a relative image draws"));
  const edit = '[data-testid="mem-log"] [data-kind="edit"]:has([data-file="files/rack.svg"])';
  await page.click(`${edit} [data-file="topics/nas-layout.md"]`);
  ok(await until(() => document.querySelector('[data-testid="mem-diff"]')?.textContent.includes("+Second shelf: four 8 TB disks")), say("one file's diff"));
  await page.click(`${edit} [data-file="files/rack.svg"]`);
  ok(await until(() => { const s = [...document.querySelectorAll('[data-testid="mem-side"]')]; return s.length === 2 && s.every((i) => i.complete && i.naturalWidth > 0) && s[0].src !== s[1].src; }), say("a changed image draws before and after"));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/memory-history.${name}.png` });
  ok(await page.locator('[data-kind="fold"]:has-text("Naming") [data-act="revert"]').textContent() === "Redo", say("an undone change offers Redo"));
  await page.click('[data-kind="fold"]:has-text("Storage layout") [data-act="revert"]');
  ok(await until(() => document.querySelector('[data-testid="mem-note"]')?.textContent.includes("conflict: MEMORY.md")), say("an undo over a later edit is refused, naming the file"));
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/second.md", "# Second\n\nx\n"), id);
  ok(await until(() => !!document.querySelector('[data-path="topics/second.md"]')), say("a new file is listed"));

  // A version, and a removal.
  await page.click('[data-testid="ideas-versions-toggle"]');
  ok(await until(() => document.querySelectorAll('[data-testid="ideas-version"]').length === 2), say("the file's changes are its versions"));
  const v = page.locator('[data-testid="ideas-version"]', { hasText: "fold: Storage layout" }).locator("button");
  await v.click(); await v.click();
  await page.waitForTimeout(400);
  ok(await page.evaluate((s) => document.querySelector(s).textContent.includes("Second shelf: not bought yet"), ed), say("a version restores"));
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/nas-layout.md", null), id);
  ok(await until(() => /removed|deleted|gone/i.test(document.querySelector(".ix-doc")?.textContent ?? "")), say("a removed file is said"));
  ok(await page.evaluate((s) => document.querySelector(s).textContent.includes("Second shelf"), ed), say("and its text stays on screen"));
  // The chats' rows and the memory's tell one story.
  const both = await page.evaluate(async (id) => {
    const { invoke } = await import("/src/ipc.ts");
    const chats = (await invoke("workspace_chats", { id })).chats;
    const folds = chats.filter((c) => c.fold?.state === "done");
    const shown = await Promise.all(folds.map((c) => invoke("workspace_memory_diff", { id, commit: c.fold.commit }).then((d) => d.change.subject === `fold: ${c.title}` && d.change.files.some((f) => `memory/${f.path}` === c.note) && d.change.added === c.fold.lines.added)));
    const notes = chats.filter((c) => c.note_exists).map((c) => c.title);
    const before = (await invoke("workspace_memory_log", { id, n: 1 })).changes[0].commit;
    window.__mock.memoryEdit(id, "topics/second.md", "# Second\n\nby hand\n");
    await invoke("workspace_chat_start", { id });
    const top = (await invoke("workspace_memory_log", { id, n: 1 })).changes[0];
    const gone = await invoke("workspace_remove", { id });
    return { n: folds.length, shown, undone: folds.filter((c) => c.fold.reverted).map((c) => c.title), notes, committed: top.commit !== before && top.subject, moved: gone.moved_to?.endsWith(`.removed/${id}`) };
  }, id);
  ok(both.n === 2 && both.shown.every(Boolean), say(`each folded chat's record names its commit and note (${JSON.stringify(both)})`));
  ok(both.undone.join() === "Naming" && both.notes.join() === "Storage layout", say("the undone fold's note is gone, the other's is there"));
  ok(/^Edited: .*topics/.test(both.committed || ""), say("a chat's start commits what was dirty"));
  ok(both.moved === true, say("a removal moves its folder"));
  ok(errors.length === 0, say(`no page errors ${errors.join(" | ")}`));
  await browser.close();
}
console.log(failed ? `\n${failed} FAILED` : "\nall ok");
process.exit(failed ? 1 : 0);
