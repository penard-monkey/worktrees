// Browser gate for a workspace's memory on screen (docs/proposals/
// workspace-view-design.md §3.1, §4.4, §4.5: WorkspaceMemory.tsx on memoryDocs),
// in Chromium AND WebKit against the mock.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/workspace-memory-browser.mjs
//
// What it holds: the chip and its purple dot; the Overview's column (the index as
// it reads, cut, what last changed it, Diff, Undo); the Memory tab in Read with no
// editor fetched, the file list at two widths, a filter, links in place with ‹ ›,
// a missing link dim, an idea: link, images, a preview; History filtered and whole,
// one file at a time, images before and after, Undo in two presses, Redo, a refusal
// naming files; no memory and Make; a memory that cannot be read; a long index;
// 200 files; all of it with Ideas off; layout and six themes.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const tab = (page, t) => page.click(`[data-testid=ws-tab][data-tab=${t}]`);
const isTab = (page, t) => until(page, (t) => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === t, t, 2000);
const isDoc = (page, id) => until(page, (id) => { const d = document.querySelector("[data-testid=main-doc]"); return !!d && !d.hidden && d.dataset.doc === id; }, id);
const read = (page) => page.evaluate(() => document.querySelector("[data-testid=main-doc] [data-testid=ideas-read]")?.innerText ?? null);
const files = (page) => page.evaluate(() => [...document.querySelectorAll("[data-testid=memory-file]")].map((b) => b.dataset.path));
const open = async (page, path) => { await page.click(`[data-testid=memory-file][data-path='${path}']`); return isDoc(page, path); };
const fetched = (page) => page.evaluate(() => performance.getEntriesByType("resource").some((r) => /codemirror|\/doc\/editor\.ts|\/assets\/editor-/.test(r.name)));
const history = async (page) => { if (await page.$("[data-testid=main-doc] [data-testid=ideas-versions-toggle]")) await page.click("[data-testid=main-doc] [data-testid=ideas-versions-toggle]"); else { await page.click("[data-testid=main-doc] [data-testid=doc-menu-open]"); await page.click("[data-testid=doc-menu-history]"); } await page.waitForSelector("[data-testid=memory-history]"); };
const histRows = (page) => page.evaluate(() => [...document.querySelectorAll("[data-testid=memory-log-row]")].map((r) => r.dataset.kind).join());

async function boot(page, betas = ["beta_workspace_chats"], query = "") {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", ...Object.fromEntries(betas.map((b) => [b, true])) });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0&memtick=150${query}`);
  await page.waitForSelector("[data-testid=ws-doc-memory]");
  return (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
}
const name = (page, n = "Everything") => page.locator("[data-testid=ws-name]", { hasText: n }).first().click();

async function overview(page, tag, eng) {
  const ws = await boot(page);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-doc-memory] [data-testid=memory-fresh]")) && !(await page.locator("[data-testid=ws-doc-memory]").first().evaluate((e) => e.classList.contains("none"))), `${tag} the Memory chip has something behind it, and the purple dot: it changed since you looked`);
  await name(page);
  await page.waitForSelector("[data-testid=memory-excerpt]");
  ok(await page.evaluate(() => { const e = document.querySelector("[data-testid=memory-excerpt]"); const h1 = e.querySelector(".md-h1"), h2 = e.querySelector(".md-h2"), p = e.querySelector("p"); const px = (x) => parseFloat(getComputedStyle(x).fontSize); return h1.getBoundingClientRect().height === 0 && px(h2) <= px(p) * 1.15; }), `${tag} its headings are a card's, and its own title is not drawn twice`);
  ok(/What this workspace is for/.test(await page.locator("[data-testid=memory-excerpt]").innerText()) && /MEMORY\.md · [\d.]+ (B|KB) · 5 files/.test(await page.locator("[data-testid=ws-ov-card-memory] .wsf-card-side").innerText()), `${tag} the Overview has the index as it reads, its size and the file count`);
  ok(/Edited by hand 4h ago/.test(await page.locator("[data-testid=memory-last]").innerText()) && /4 files/.test(await page.locator("[data-testid=memory-last]").innerText()), `${tag} what last changed it, and how much`);
  ok(await page.evaluate(() => { const e = document.querySelector("[data-testid=memory-excerpt]"); return e.scrollTop === 0 && getComputedStyle(e).overflowY === "hidden"; }), `${tag} never in a scroller of its own`);
  // A long one is cut, and says where the rest is.
  const long = "# Memory: Everything\n\n" + Array.from({ length: 90 }, (_, i) => `- 2026-11-${String((i % 28) + 1).padStart(2, "0")} Decision number ${i}.`).join("\n") + "\n";
  await page.evaluate(([id, t]) => window.__mock.memoryEdit(id, "MEMORY.md", t), [ws, long]);
  ok(await until(page, () => document.querySelector("[data-testid=memory-excerpt]")?.classList.contains("cut") && /Read all of it in Memory/.test(document.querySelector("[data-testid=memory-read-all]").textContent)), `${tag} a long one is cut where the column ends: Read all of it in Memory`);
  ok(await page.evaluate(() => { const c = document.querySelector("[data-testid=ws-ov-card-memory]").getBoundingClientRect(); return c.bottom <= innerHeight + 4; }), `${tag} …within the window`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsm-overview-${eng}.png` });
  await page.click("[data-testid=memory-read-all]");
  ok(await isTab(page, "memory") && await isDoc(page, "MEMORY.md"), `${tag} which opens the index on the Memory tab`);
  await tab(page, "overview");
  await page.waitForTimeout(200);
  const dot = () => page.evaluate(() => !!document.querySelector("[data-testid=ws-docs]").querySelector("[data-testid=memory-fresh]"));
  ok(!(await dot()), `${tag} and the dot goes, and stays gone: it was looked at`);
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/nas-layout.md", "# NAS layout\n\nChanged by an agent.\n"), ws);
  await page.waitForTimeout(400);
  // ── Diff, from the line ──
  await page.click("[data-testid=memory-last-diff]");
  ok(await isTab(page, "memory") && await until(page, () => !!document.querySelector("[data-testid=memory-change]")), `${tag} its Diff opens that change in History, on the Memory tab`);
  await page.click("[data-testid=memory-hist-close]");
  ok(await until(page, () => !document.querySelector("[data-testid=memory-history]") && !!document.querySelector("[data-testid=main-doc] [data-testid=ideas-read]")), `${tag} Close is the document again`);
}

async function tabs(page, tag, eng) {
  const ws = await boot(page, ["beta_workspace_chats", "beta_ideas"]);
  await page.locator("[data-testid=ws-doc-memory]").first().click();
  ok(await isDoc(page, "MEMORY.md") && /What this workspace is for/.test(await read(page)), `${tag} the Memory tab opens the index, in Read`);
  await page.waitForTimeout(400);
  ok(!(await fetched(page)), `${tag} …and no editor is fetched`);
  ok(await page.evaluate(() => { const s = document.querySelector("[data-testid=ws-side]").getBoundingClientRect(), d = document.querySelector("[data-testid=main-doc]").getBoundingClientRect(); return s.right <= d.left + 1; }), `${tag} its files are a column beside it`);
  const list = await files(page);
  ok(list[0] === "MEMORY.md" && list.filter((p) => p.startsWith("notes/")).length === 2 && list.includes("topics/nas-layout.md") && list.includes("files/rack.svg"), `${tag} the index, then notes, topics, files (${list})`);
  ok((await page.locator("[data-testid=main-doc] .ix-title").innerText()) === "MEMORY.md" && await page.locator("[data-testid=main-doc] [data-testid=main-doc-float]").count() === 0, `${tag} its name is the file's, and it does not float yet`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsm-tab-${eng}.png` });
  // ── links ──
  await page.locator("[data-testid=main-doc] [data-testid=ideas-read] a", { hasText: "NAS layout" }).click();
  ok(await isDoc(page, "topics/nas-layout.md") && await page.locator("[data-testid=main-doc] .wsm-crumb").innerText() === "topics ▸", `${tag} a relative link opens that file in place, with its folder named`);
  ok(await until(page, () => { const i = document.querySelector("[data-testid=main-doc] [data-testid=doc-image]"); return i && i.complete && i.naturalWidth > 0; }), `${tag} an image is drawn in Read`);
  await page.locator("[data-testid=main-doc] [data-testid=ideas-read] a", { hasText: "storage layout" }).click();
  ok(await until(page, () => /^notes\//.test(document.querySelector("[data-testid=main-doc]")?.dataset.doc ?? "")), `${tag} a link up and over resolves from its own file`);
  await page.click("[data-testid=memory-back]");
  ok(await isDoc(page, "topics/nas-layout.md"), `${tag} ‹ is the file before`);
  await page.click("[data-testid=memory-back]");
  ok(await isDoc(page, "MEMORY.md") && await page.locator("[data-testid=memory-back]").isDisabled(), `${tag} …to the first`);
  await page.click("[data-testid=memory-forward]");
  ok(await isDoc(page, "topics/nas-layout.md"), `${tag} › goes on`);
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/links.md", "# Links\n\n[gone](nowhere.md) · [out](../../etc/passwd) · [an idea](idea:01JC8Z4K3M9QX2T7VB00000901) · [here](nas-layout.md)\n\n![far](https://example.com/x.png)\n"), ws);
  ok(await until(page, () => !!document.querySelector("[data-testid=memory-file][data-path='topics/links.md']")), `${tag} a file an agent wrote is listed as it lands`);
  await open(page, "topics/links.md");
  const links = await page.evaluate(() => [...document.querySelectorAll("[data-testid=main-doc] [data-testid=ideas-read] a")].map((a) => [a.textContent, a.classList.contains("gone"), a.title]));
  ok(links[0][1] && /There is no topics\/nowhere\.md/.test(links[0][2]) && links[1][1] && /Outside this memory/.test(links[1][2]) && !links[2][1] && !links[3][1], `${tag} a missing link is dim and says so, and one outside (${JSON.stringify(links)})`);
  ok(/Remote image, not loaded/.test(await read(page)) && await page.evaluate(() => !performance.getEntriesByType("resource").some((r) => /example\.com/.test(r.name))), `${tag} a remote image is never fetched`);
  await page.waitForTimeout(800); // past the history's coalesce window
  await page.locator("[data-testid=main-doc] [data-testid=ideas-read] a", { hasText: "an idea" }).click();
  ok(await isTab(page, "ideas") && await isDoc(page, "01JC8Z4K3M9QX2T7VB00000901"), `${tag} an idea: link opens the idea, on the Ideas tab`);
  await page.waitForTimeout(800);
  await page.locator("[data-testid=nav-back]:visible").first().click();
  ok(await isTab(page, "memory") && await isDoc(page, "topics/links.md"), `${tag} and Back is the file it was in`);
  // ── what is not a text ──
  await page.click("[data-testid=memory-file][data-path='files/rack.svg']");
  ok(await until(page, () => { const i = document.querySelector("[data-testid=memory-preview] img"); return i && i.complete && i.naturalWidth > 0; }) && /files\/rack\.svg · /.test(await page.locator("[data-testid=memory-preview]").innerText()) && !!(await page.$("[data-testid=memory-reveal]")), `${tag} an image has a preview, its size, and Show in Finder`);
  await open(page, "MEMORY.md");
  ok(!(await page.$("[data-testid=memory-preview]")), `${tag} …which a text replaces`);
  // ── the filter ──
  await page.fill("[data-testid=memory-filter]", "nas");
  ok(JSON.stringify(await files(page)) === '["topics/nas-layout.md"]', `${tag} the filter`);
  await page.fill("[data-testid=memory-filter]", "");
  // ── narrow ──
  await page.setViewportSize({ width: 1000, height: 800 }); // 612 of room
  ok(await until(page, () => /Files · \d+/.test(document.querySelector("[data-testid=ws-side-toggle]")?.textContent ?? "") && !document.querySelector("[data-testid=ws-side]")), `${tag} under 720 the list is a Files button`);
  await page.click("[data-testid=ws-side-toggle]");
  ok(await open(page, "topics/nas-layout.md"), `${tag} …that opens it, over the document`);
  await page.setViewportSize({ width: 1440, height: 900 });
  // ── Edit loads the editor, and a change an agent makes is read ──
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  ok(await until(page, () => !!document.querySelector("[data-testid=main-doc] .cm-content")) && await fetched(page), `${tag} Edit fetches the editor, then`);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-read]");
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/nas-layout.md", "# NAS layout\n\nAGENT SAYS\n"), ws);
  ok(await until(page, () => /AGENT SAYS/.test(document.querySelector("[data-testid=main-doc] [data-testid=ideas-read]")?.innerText ?? "")), `${tag} an agent's edit is read as it lands`);
  // ── a chat's note, from the Chats tab ──
  await tab(page, "chats");
  const noted = (await call(page, "workspace_chats", { id: ws })).chats;
  ok(noted.some((c) => c.note === "") && (await page.locator("[data-testid=ws-chats-note]").count()) === noted.filter((c) => c.note && c.note_exists).length, `${tag} a chat that was never folded has no note to open`);
  await page.locator("[data-testid=ws-chats-note]").first().click();
  ok(await isTab(page, "memory") && await until(page, () => /^notes\//.test(document.querySelector("[data-testid=main-doc]")?.dataset.doc ?? "")), `${tag} a folded chat's note opens from the Chats tab`);
}

async function hist(page, tag, eng) {
  const ws = await boot(page);
  await page.locator("[data-testid=ws-doc-memory]").first().click();
  await isDoc(page, "MEMORY.md");
  await history(page);
  ok(await until(page, () => document.querySelectorAll("[data-testid=memory-log-row]").length > 0) && await histRows(page) === "edit,fold,undo,fold,made", `${tag} the index's changes, newest first (${await histRows(page)})`);
  ok(await page.evaluate(() => getComputedStyle(document.querySelector("[data-testid=main-doc] .ix-body")).display === "none"), `${tag} History takes the document's place`);
  ok(await page.evaluate(() => [...document.querySelectorAll(".wsm-row")].every((r) => { const n = r.querySelector(".wsm-row-n"), d = r.querySelector(".wsm-row-d").getBoundingClientRect(); return d.right >= r.getBoundingClientRect().right - 20 && n.scrollWidth <= n.clientWidth + 1; })), `${tag} a change's subject takes the row's room`);
  await page.click("[data-testid=memory-hist-close]");
  await page.click("[data-testid=memory-file][data-path='topics/nas-layout.md']");
  await isDoc(page, "topics/nas-layout.md");
  if (!(await page.$("[data-testid=memory-history]"))) await history(page);
  ok(await until(page, () => /Changes to topics\/nas-layout\.md/.test(document.querySelector("[data-testid=memory-history]")?.textContent ?? "")), `${tag} another file's are its own`);
  const mine = (await histRows(page)).split(",").length;
  await page.click("[data-testid=memory-hist-all]");
  ok(await until(page, (n) => document.querySelectorAll("[data-testid=memory-log-row]").length > n, mine), `${tag} All changes is the folder's`);
  // ── one change, one file ──
  await page.locator("[data-testid=memory-log-row][data-kind=edit]").first().click();
  await page.waitForSelector("[data-testid=memory-change]");
  const cf = await page.locator("[data-testid=memory-change-file]").evaluateAll((b) => b.map((x) => x.dataset.path));
  ok(cf.length === 4 && await page.locator("[data-testid=memory-diff]").count() === 1, `${tag} a change lists its files, and shows one diff (${cf})`);
  await page.click("[data-testid=memory-change-file][data-path='MEMORY.md']");
  ok(await until(page, () => !!document.querySelector("[data-testid=memory-diff] .wsm-dl.add")), `${tag} a text's lines`);
  ok(await page.evaluate(() => { const r = document.querySelector(".wsm-dl.add"); const c = getComputedStyle(r).backgroundColor; return !/, 0\)|\/ 0\)|transparent/.test(c); }), `${tag} …tinted over the panel`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsm-diff-${eng}.png` });
  await page.click("[data-testid=memory-change-file][data-path='files/rack.svg']");
  ok(await until(page, () => document.querySelectorAll("[data-testid=memory-diff-image]").length === 2 && [...document.querySelectorAll("[data-testid=memory-diff-image]")].every((i) => i.naturalWidth > 0)), `${tag} a changed image, before and after`);
  // ── Undo, Redo ──
  ok(await page.locator("[data-testid=memory-revert]").innerText() === "Undo", `${tag} it has Undo`);
  const before = (await call(page, "workspace_memory_log", { id: ws })).changes.length;
  await page.click("[data-testid=memory-revert]");
  ok(/Undo this change\?/.test(await page.locator("[data-testid=memory-revert]").innerText()) && (await call(page, "workspace_memory_log", { id: ws })).changes.length === before, `${tag} …in two presses`);
  await page.click("[data-testid=memory-revert]");
  ok(await until(page, () => /Edited by hand|was undone/.test(document.querySelector("[data-testid=memory-change]")?.textContent ?? "") && document.querySelector("[data-testid=memory-change]").querySelector(".wsm-change-h.undo")), `${tag} the second undoes it, with a new change, shown`);
  ok((await call(page, "workspace_memory_log", { id: ws })).changes[0].kind === "undo", `${tag} …nothing dropped`);
  await page.click("[data-testid=memory-hist-back]");
  await until(page, () => document.querySelectorAll("[data-testid=memory-log-row]").length > 0);
  ok(await until(page, () => !!document.querySelector("[data-testid=memory-log-row][data-kind=edit].undone")), `${tag} the undone one is marked in the list`);
  await page.locator("[data-testid=memory-log-row][data-kind=edit].undone").first().click();
  ok(await until(page, () => document.querySelector("[data-testid=memory-revert]")?.textContent === "Redo"), `${tag} and has Redo`);
  await page.click("[data-testid=memory-revert]"); await page.click("[data-testid=memory-revert]");
  ok(await until(page, () => !!document.querySelector(".wsm-change-h.redo")), `${tag} …which puts it back, as a change (${await page.evaluate(() => document.querySelector("[data-testid=memory-conflict]")?.textContent + " | " + document.querySelector("[data-testid=memory-revert]")?.textContent)})`);
  // ── a refusal ──
  await page.click("[data-testid=memory-hist-back]");
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/nas-layout.md", "# NAS layout\n\nLater words.\n"), ws);
  await page.waitForTimeout(400);
  await page.locator("[data-testid=memory-log-row][data-kind=fold]").first().click();
  await page.waitForSelector("[data-testid=memory-change]");
  await page.click("[data-testid=memory-revert]"); await page.click("[data-testid=memory-revert]");
  ok(await until(page, () => /It was not undone: .*topics\/nas-layout\.md.* changed since/.test(document.querySelector("[data-testid=memory-conflict]")?.textContent ?? "")), `${tag} an Undo that meets later edits is refused whole, naming the files`);
  await page.click("[data-testid=memory-change-file][data-path='topics/nas-layout.md']");
  const path = "topics/nas-layout.md";
  await page.click("[data-testid=memory-change-open]");
  ok(await isDoc(page, path) && !(await page.$("[data-testid=memory-history]")), `${tag} Open opens the file`);
}

async function states(page, tag) {
  const ws = await boot(page, ["beta_workspace_chats"], "&memory=none");
  ok(await page.locator("[data-testid=ws-doc-memory]").first().evaluate((e) => e.classList.contains("none")) && !(await page.$("[data-testid=memory-fresh]")), `${tag} no memory: a dashed chip, no dot`);
  await name(page);
  ok(await until(page, () => /No memory yet\. It starts with the first chat\./.test(document.querySelector("[data-testid=ws-ov-card-memory]")?.textContent ?? "")), `${tag} the Overview says so`);
  ok(await page.evaluate(() => document.querySelector("[data-testid=ws-ov-card-memory]").getBoundingClientRect().height < 180), `${tag} …in a card as tall as its words`);
  await page.locator("[data-testid=ws-doc-memory]").first().click();
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-no-memory]")) && !(await page.$("[data-testid=ws-side]")), `${tag} and the tab, with no list`);
  await page.click("[data-testid=ws-make-memory]");
  ok(await isDoc(page, "MEMORY.md") && /What this workspace is for/.test(await read(page)), `${tag} Make memory makes it, and opens the index`);
  ok(!(await page.$("[data-testid=memory-fresh]")), `${tag} …which is nothing new to look at`);
  // ── a memory that cannot be read ──
  await page.evaluate(() => { const t = window.__TAURI_INTERNALS__, i = t.invoke; window.__broken = true; t.invoke = (c, a) => (c === "workspace_memory" && window.__broken ? Promise.reject("its folder is a symlink") : i(c, a)); });
  await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# x\n"), ws);
  await tab(page, "overview");
  ok(await until(page, () => /could not be read[\s\S]*its folder is a symlink/.test(document.querySelector("[data-testid=ws-ov-card-memory]")?.textContent ?? "")) && !/No memory yet/.test(await page.locator("[data-testid=ws-ov-card-memory]").innerText()), `${tag} one that cannot be read says why, and is not "no memory"`);
  await page.evaluate(() => { window.__broken = false; });
  await page.click("[data-testid=memory-retry]");
  ok(await until(page, () => !!document.querySelector("[data-testid=memory-excerpt]")), `${tag} Retry reads it`);
  // ── a long index ──
  await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# Memory\n\n" + "A line of the index, long enough to count.\n".repeat(560)), ws);
  await tab(page, "memory");
  ok(await until(page, () => /The index is long \(\d+ KB\)\. Every chat reads all of it\./.test(document.querySelector("[data-testid=memory-long]")?.textContent ?? "")) && await page.locator("[data-testid=memory-long] button").isDisabled(), `${tag} a long index says so; Tidy… is not here yet`);
  // ── two hundred files ──
  await page.evaluate((id) => { for (let i = 0; i < 200; i++) window.__mock.memoryEdit(id, `notes/2026-${String(1 + (i % 10)).padStart(2, "0")}-${String(1 + (i % 28)).padStart(2, "0")}-n${i}.md`, `# Note ${i}\n`); }, ws);
  ok(await until(page, () => document.querySelectorAll("[data-testid=memory-group]").length === 10, null, 10000), `${tag} 200 notes fold by month`);
  ok((await files(page)).length === 21, `${tag} …the newest month open alone (${(await files(page)).length})`);
  await page.fill("[data-testid=memory-filter]", "n123");
  ok(JSON.stringify(await files(page)) === JSON.stringify((await files(page)).filter((p) => /n123\.md$/.test(p))) && (await files(page)).length === 1, `${tag} and the filter finds one of them`);
  // ── off ──
  await page.keyboard.press("Meta+Comma");
  await page.click("[data-testid=beta-workspace-chats]");
  ok(await until(page, () => !document.querySelector("[data-testid=main-doc]") && !document.querySelector("[data-testid=ws-doc-memory]")), `${tag} turned off, the open file is closed and the chip is gone`);
}

/** Slice 5: a person writes in it, beside chats that may. */
async function edit(page, tag, eng) {
  const ws = await boot(page, ["beta_workspace_chats", "beta_ideas"]);
  await call(page, "workspace_link_idea", { id: ws, idea: "01JC8Z4K3M9QX2T7VB00000901" });
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.locator("[data-testid=ws-doc-memory]").first().click();
  await isDoc(page, "MEMORY.md");
  ok(!(await page.$("[data-testid=memory-may-edit]")), `${tag} in Read nothing is said of live chats`);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  ok(await until(page, () => /2 chats are live and may edit this file too\./.test(document.querySelector("[data-testid=memory-may-edit]")?.textContent ?? "")), `${tag} in Edit, with chats live, it says they may edit this too`);
  const ed = page.locator("[data-testid=main-doc] .cm-content");
  await ed.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type("\nMINE");
  ok(await until(page, (id) => window.__mock.memoryWrites().some((w) => w.path === "MEMORY.md" && /MINE/.test(w.content)), ws), `${tag} …and does not block: typing saves`);
  await page.click("[data-testid=memory-may-edit-ok]");
  ok(!(await page.$("[data-testid=memory-may-edit]")), `${tag} Dismiss`);
  await until(page, () => document.querySelector("[data-testid=main-doc] [data-testid=ideas-save]")?.dataset.state === "clean");
  await page.waitForTimeout(400);
  ok(!(await page.$("[data-testid=memory-under]")), `${tag} your own save is not a change under you`);

  // ── [[ ──
  await ed.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type("\nSee [[");
  ok(await until(page, () => document.querySelectorAll("[data-testid=ideas-link-pick]").length >= 5), `${tag} [[ offers`);
  const rows = await page.locator("[data-testid=ideas-link-pick]").evaluateAll((b) => b.map((x) => x.textContent));
  ok(/idea$/.test(rows.at(-1)) && rows.slice(0, -1).every((r) => /(note|topic|image|file)$/.test(r)) && !rows.some((r) => /^MEMORY/.test(r)), `${tag} …this memory's files first, then the linked idea (${rows.join(" | ")})`);
  await page.keyboard.type("nas");
  await until(page, () => document.querySelectorAll("[data-testid=ideas-link-pick]").length === 1);
  await page.keyboard.press("Enter");
  ok(await until(page, () => window.__mock.memoryWrites().some((w) => /See \[NAS layout\]\(topics\/nas-layout\.md\)/.test(w.content))), `${tag} and writes a plain markdown link`);
  await page.locator("[data-testid=main-doc] .cm-line", { hasText: "See " }).locator("span", { hasText: "NAS layout" }).first().click({ modifiers: ["ControlOrMeta"] });
  ok(await isDoc(page, "topics/nas-layout.md"), `${tag} ⌘-click on it follows it`);

  // ── New file ── (from Read, so it is the new file's own Edit that is seen)
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-read]");
  await page.click("[data-testid=memory-new]");
  await page.fill("[data-testid=memory-new-name]", "../x");
  ok(/not a file name/.test(await page.locator("[data-testid=memory-new-path]").innerText()) && await page.locator("[data-testid=memory-new-make]").isDisabled(), `${tag} New file refuses what is not a name`);
  await page.fill("[data-testid=memory-new-name]", "Guest access");
  ok((await page.locator("[data-testid=memory-new-path]").innerText()) === "topics/guest-access.md", `${tag} …and says where it goes`);
  await page.keyboard.press("Enter");
  ok(await isDoc(page, "topics/guest-access.md") && await until(page, () => /# Guest access/.test(document.querySelector("[data-testid=main-doc] .cm-content")?.innerText ?? "")), `${tag} it is made, and opens in Edit`);
  await page.click("[data-testid=memory-new]");
  await page.fill("[data-testid=memory-new-name]", "Guest access");
  await page.keyboard.press("Enter");
  ok(await until(page, () => /already a file/.test(document.querySelector("[data-testid=memory-new-path]")?.textContent ?? "")), `${tag} one that is there is refused, in words`);
  await page.keyboard.press("Escape");
  ok(await until(page, () => !document.querySelector("[data-testid=memory-new-name]")), `${tag} Escape puts it away`);

  // ── a dropped image, a pasted one, one too big ──
  const drop = (kind, size, name) => page.evaluate(([kind, size, name]) => {
    const png = atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==");
    const bytes = size ? new Uint8Array(size) : Uint8Array.from(png, (c) => c.charCodeAt(0));
    const dt = new DataTransfer();
    dt.items.add(new File([bytes], name, { type: "image/png" }));
    const el = document.querySelector("[data-testid=main-doc] .cm-content"), r = el.getBoundingClientRect();
    const e = kind === "drop" ? new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt, clientX: r.x + 60, clientY: r.y + 40 }) : new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: dt });
    el.dispatchEvent(e);
    return e.defaultPrevented;
  }, [kind, size, name]);
  await ed.click();
  ok(await drop("drop", 0, "wiring.png"), `${tag} a dropped image is taken`);
  ok(await until(page, () => window.__mock.memoryWrites().some((w) => w.path === "topics/guest-access.md" && /!\[wiring\]\(\.\.\/files\/wiring\.png\)/.test(w.content))), `${tag} …kept under files, and linked from where it fell`);
  ok(await until(page, () => !!document.querySelector("[data-testid=memory-file][data-path='files/wiring.png']")), `${tag} …and listed`);
  await drop("paste", 0, "image.png");
  ok(await until(page, () => window.__mock.memoryWrites().some((w) => /!\[pasted-[\d-]+\]\(\.\.\/files\/pasted-[\d-]+\.png\)/.test(w.content))), `${tag} a pasted one is kept under a name of its own`);
  await drop("drop", 5 * 1024 * 1024, "huge.png");
  ok(await until(page, () => /huge\.png is 5\.0 MB: a memory takes files up to 4 MB\./.test(document.querySelector("[data-testid=main-doc] [data-testid=ideas-notice]")?.textContent ?? "")) && !(await files(page)).includes("files/huge.png"), `${tag} 5 MB is refused in words`);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-read]");
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=main-doc] [data-testid=doc-image]")].some((i) => i.naturalWidth > 0)), `${tag} Read draws it`);

  // ── changed under you ──
  await open(page, "topics/nas-layout.md");
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/nas-layout.md", "# NAS layout\n\nONE NEW LINE\n"), ws);
  ok(await until(page, () => /Changed on disk\. 2 chats are live\./.test(document.querySelector("[data-testid=memory-under]")?.textContent ?? "")) && /ONE NEW LINE/.test(await read(page)), `${tag} changed with nothing unsaved: it is there, and it is said`);
  await page.click("[data-testid=memory-under-diff]");
  ok(await until(page, () => /ONE NEW LINE/.test(document.querySelector(".wsm-under .wsm-dl.add")?.textContent ?? "") && !!document.querySelector(".wsm-under .wsm-dl.del")), `${tag} …with the diff`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsm-edit-${eng}.png` });
  await page.click("[data-testid=memory-under-ok]");
  ok(!(await page.$("[data-testid=memory-under]")), `${tag} …until dismissed`);

  // ── both at once ──
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await ed.click();
  await page.keyboard.type("UNSAVED ");
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/nas-layout.md", "# NAS layout\n\nTHEIRS\n"), ws);
  ok(await until(page, () => /Keep mine \(theirs goes to History\)/.test(document.querySelector("[data-testid=ideas-conflict]")?.textContent ?? "")) && /This file /.test(await page.locator("[data-testid=ideas-conflict]").innerText()), `${tag} with unsaved text it is the conflict, and theirs goes to History`);
  ok(!(await page.$("[data-testid=memory-under]")), `${tag} …not "changed on disk"`);
  await page.click("[data-testid=ideas-keep-mine]");
  ok(await until(page, () => window.__mock.memoryWrites().some((w) => /UNSAVED/.test(w.content) && w.keep_history)), `${tag} Keep mine saves it, theirs committed first`);
  await until(page, () => document.querySelector("[data-testid=main-doc] [data-testid=ideas-save]")?.dataset.state === "clean");
  await page.waitForTimeout(400);
  await ed.click();
  await page.keyboard.type("AGAIN ");
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/nas-layout.md", "# NAS layout\n\nTHEIRS TOO\n"), ws);
  await until(page, () => !!document.querySelector("[data-testid=ideas-conflict]"));
  await page.click("[data-testid=ideas-reload]"); await page.click("[data-testid=ideas-reload]");
  ok(await until(page, () => /THEIRS TOO/.test(document.querySelector("[data-testid=main-doc] .cm-content")?.innerText ?? "")) && !(await page.$("[data-testid=memory-under]")), `${tag} Reload theirs is your own act, and says nothing more`);
}

async function themes(page, tag, eng) {
  await boot(page, ["beta_workspace_chats", "beta_ideas"]);
  await page.locator("[data-testid=ws-doc-memory]").first().click();
  await isDoc(page, "MEMORY.md");
  for (const w of [1600, 1440, 1200, 900]) {
    await page.setViewportSize({ width: w, height: 900 });
    await page.waitForTimeout(250);
    const bad = await page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-frame] button:not(:disabled)")].filter((b) => { const q0 = b.getBoundingClientRect(); if (!q0.width) return false; b.scrollIntoView({ block: "nearest" }); const q = b.getBoundingClientRect(), hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2); return !(hit && (b === hit || b.contains(hit))); }).map((b) => b.dataset.testid || b.textContent));
    const h = await page.evaluate(() => document.querySelector("[data-testid=main-doc] .ix-head").getBoundingClientRect().height);
    ok(bad.length === 0 && h <= 45, `${tag} ${w}px: every control is reachable, the header one row${bad.length ? `: ${bad}` : ""} (${h})`);
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  for (const theme of THEMES) {
    await page.evaluate((t) => { if (t === "tokyo-night") delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t; }, theme);
    for (const t of ["memory", "overview", "history"]) {
      if (t === "history") { await tab(page, "memory"); await isDoc(page, "MEMORY.md"); if (!(await page.$("[data-testid=memory-history]"))) await history(page); await page.locator("[data-testid=memory-log-row][data-kind=edit]").first().click(); await page.click("[data-testid=memory-change-file][data-path='MEMORY.md']"); await page.waitForSelector(".wsm-dl.add"); }
      else await tab(page, t);
      await page.waitForTimeout(150);
      const low = await page.evaluate(() => {
        const parse = (c) => { let x = /^rgba?\(([^)]+)\)/.exec(c); if (x) { const p = x[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; } x = /^color\(srgb ([^)]+)\)/.exec(c); if (x) { const p = x[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; } return [0, 0, 0, 0]; };
        const over = (a, b) => [0, 1, 2].map((i) => a[i] * a[3] + b[i] * (1 - a[3])).concat(1);
        const bgOf = (el) => { const st = []; for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) st.push(c); if (c[3] >= 1) break; } let acc = [255, 255, 255, 1]; for (const c of st.reverse()) acc = over(c, acc); return acc; };
        const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
        const ratio = (el) => { const bg = bgOf(el); const [a, b] = [lum(over(parse(getComputedStyle(el).color), bg)), lum(bg)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
        return [...document.querySelectorAll(".wsm-file-n, .wsm-last-w, .wsm-add, .wsm-del, .wsm-row-n, .wsm-change-w b, .wsm-cfile-n, .wsm-dt, .wsf-card-side, .wsm-hist-t, [data-testid=ws-frame] .ctrl:not(:disabled)")]
          .filter((e) => e.textContent.trim() && e.getBoundingClientRect().width).map((e) => [e.className + ": " + e.textContent.trim().slice(0, 24), ratio(e)]).filter(([, r]) => r < 4.5);
      });
      ok(low.length === 0, `${tag} ${theme} ${t}: its words read ≥ 4.5:1${low.length ? `: ${low.slice(0, 4).map(([a, r]) => `${a} ${r.toFixed(2)}`).join("; ")}` : ""}`);
    }
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsm-${theme}-${eng}.png` });
    await page.click("[data-testid=memory-hist-close]");
  }
}

const only = process.env.ONLY?.split(",");
for (const [engine, eng] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [overview, tabs, hist, states, edit, themes]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    page.on("console", (m) => { if (m.type() === "error" && /Maximum update depth|Cannot update a component|unique "key"/.test(m.text())) errors.push(m.text().slice(0, 200)); });
    const tag = `[${eng} ${part.name}]`;
    try { await part(page, tag, eng); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsm-${eng}-${part.name}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nworkspace-memory-browser: ${failed} failure(s)` : "\nworkspace-memory-browser: all good");
process.exit(failed ? 1 : 0);
