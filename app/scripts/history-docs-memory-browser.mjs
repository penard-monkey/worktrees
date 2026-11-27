// Headless drive of the mock harness in Chromium AND WebKit: Back and Forward
// cover the document chosen in a place's Docs tab, and a workspace Memory
// file's History (the list, and one change in it).
//
//   VITE_MOCK=1 vite --port 1487 --strictPort --force          # in app/
//   PLAYWRIGHT_MODULE=…/playwright/index.mjs HIST_URL=http://localhost:1487 \
//     node app/scripts/history-docs-memory-browser.mjs          # non-zero on failure
//
// WHY THIS EXISTS. The owner's rule (2026-11-27): history records a change of
// the PRIMARY content a pane shows. Two were left out. A document chosen on
// Docs opened in Files, but the Docs entry forgot it: Back to the list showed
// nothing chosen, and both visits to it read "Docs". A Memory file's History
// was no step at all: Back from a change in it went to the file before, with
// History still open over it. Both walks below fail on v0.54.0 + #713.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.HIST_URL ?? "http://localhost:1487";
let failures = 0;
const check = (ok, m) => { console.log(`${ok ? "ok" : "not ok"} — ${m}`); if (!ok) failures++; };
// Past navHistory's COALESCE_MS (600): two pushes closer than that are one visit.
const settle = (page) => page.waitForTimeout(800);
const step = async (page, name, dir) => {
  const b = page.locator(`[data-testid="${dir < 0 ? "nav-back" : "nav-forward"}"]:visible`).first();
  const on = await b.isEnabled().catch(() => false);
  check(on, `${name}: ${dir < 0 ? "Back" : "Forward"} is there to press`);
  if (on) await b.click();
  await settle(page);
};

async function docs(engine, name) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    page.setDefaultTimeout(8000);
    const seen = () => page.evaluate(() => ({
      tab: document.querySelector(".docspane") ? "docs" : document.querySelector(".filetree") ? "files" : null,
      chosen: document.querySelector(".docs-row.sel .docs-rel")?.textContent ?? null,
      file: document.querySelector(".viewer-path")?.getAttribute("title")?.replace(/^.*\//, "") ?? null,
    }));
    const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);
    await page.goto(`${base}/`);
    await page.waitForFunction(() => !!window.__mock);
    await settle(page);
    if (!(await page.locator("li.row[data-slug]").first().isVisible())) { await page.locator('[data-testid="places-rail"]').click(); await settle(page); }
    await page.locator("li.row[data-slug]:visible").nth(1).click();
    await settle(page);
    const docsTab = () => page.locator('[data-track="dock.docs"]').first().click().then(() => settle(page));
    // Two documents the mock's disk holds: Back to one it does not hold opens
    // nothing and says so, as any Files entry does (`file_readable`).
    const choose = (rel) => page.locator(".docs-row", { has: page.locator(".docs-rel", { hasText: rel }) }).first().click().then(() => settle(page));

    await docsTab();
    check(same(await seen(), { tab: "docs", chosen: null, file: null }), `${name}: the Docs list, nothing chosen: ${JSON.stringify(await seen())}`);
    await choose("README.md");
    const readme = await seen();
    check(readme.tab === "files" && readme.file === "README.md", `${name}: README chosen, read in Files: ${JSON.stringify(readme)}`);
    await docsTab();
    check((await seen()).chosen === "README.md", `${name}: back on Docs, README is the one chosen: ${JSON.stringify(await seen())}`);
    await choose("live-docs/task_plan.md");
    check((await seen()).file === "task_plan.md", `${name}: the plan chosen, read in Files: ${JSON.stringify(await seen())}`);
    const back = await page.locator('[data-testid="nav-back"]:visible').first().getAttribute("title");
    check(/Docs · task_plan\.md/.test(back ?? ""), `${name}: Back names the list and the document chosen in it: ${back}`);

    await step(page, name, -1);
    check(same(await seen(), { tab: "docs", chosen: ".planning/live-docs/task_plan.md", file: null }), `${name}: Back → the list, the plan chosen: ${JSON.stringify(await seen())}`);
    await step(page, name, -1);
    check((await seen()).tab === "files" && (await seen()).file === "README.md", `${name}: Back → README in Files: ${JSON.stringify(await seen())}`);
    await step(page, name, -1);
    check(same(await seen(), { tab: "docs", chosen: "README.md", file: null }), `${name}: Back → the list, README chosen: ${JSON.stringify(await seen())}`);
    await step(page, name, 1);
    check((await seen()).file === "README.md", `${name}: Forward → README again: ${JSON.stringify(await seen())}`);
    await step(page, name, 1);
    check(same(await seen(), { tab: "docs", chosen: ".planning/live-docs/task_plan.md", file: null }), `${name}: Forward → the list, the plan chosen: ${JSON.stringify(await seen())}`);
    await step(page, name, 1);
    check((await seen()).file === "task_plan.md", `${name}: Forward → the plan again: ${JSON.stringify(await seen())}`);

    // The plan leaves the place's index (the mock's flat tree has none):
    // Back skips the list entry that chose it, as it skips any dead entry.
    await step(page, name, -1); // the list, the plan chosen; re-index there
    await page.evaluate(() => history.replaceState(null, "", "/?docs=flat"));
    await page.locator('[data-track="docs.refresh"]').first().click();
    await settle(page);
    check(!(await page.locator(".docs-rel", { hasText: "live-docs/task_plan.md" }).count()), `${name}: the plan is gone from the index`);
    await step(page, name, 1); // the plan in Files
    await step(page, name, -1);
    check((await seen()).file === "README.md", `${name}: Back skips the list that chose a document now gone: ${JSON.stringify(await seen())}`);
  } catch (e) {
    check(false, `${name}: ${e.message.split("\n")[0]}`);
  } finally {
    await browser.close();
  }
}

async function memory(engine, name) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    page.setDefaultTimeout(8000);
    await page.addInitScript((s) => { if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s)); },
      { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", beta_workspace_chats: true });
    const seen = () => page.evaluate(() => ({
      doc: document.querySelector("[data-testid=main-doc]:not([hidden])")?.dataset.doc ?? null,
      hist: !!document.querySelector("[data-testid=memory-history]"),
      change: document.querySelector("[data-testid=memory-change]")?.dataset.commit ?? null,
    }));
    const same = (got, want) => JSON.stringify(got) === JSON.stringify(want);
    const history = async () => {
      if (await page.$("[data-testid=main-doc] [data-testid=ideas-versions-toggle]")) await page.click("[data-testid=main-doc] [data-testid=ideas-versions-toggle]");
      else { await page.click("[data-testid=main-doc] [data-testid=doc-menu-open]"); await page.click("[data-testid=doc-menu-history]"); }
      await page.waitForSelector("[data-testid=memory-log-row]");
      await settle(page);
    };
    await page.goto(`${base}/?ideas=seed&workspaces=1&folding=0`);
    // The workspace's Memory tab, through its overview: the nav card has no
    // document chips since #712.
    await page.waitForSelector("[data-testid=ws-name]");
    await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
    await page.waitForSelector("[data-workspace-panel=memory]");
    await page.click("[data-workspace-panel=memory]");
    await settle(page);
    await page.click("[data-testid=memory-file][data-path='topics/nas-layout.md']");
    await settle(page);
    const file = await seen();
    check(same(file, { doc: "topics/nas-layout.md", hist: false, change: null }), `${name}: reading nas-layout: ${JSON.stringify(file)}`);
    await history();
    const list = await seen();
    check(list.hist && !list.change, `${name}: its History, the list: ${JSON.stringify(list)}`);
    const back = await page.locator('[data-testid="nav-back"]:visible').first().getAttribute("title");
    check(/Memory$|Memory \(/.test(back ?? ""), `${name}: Back names the file before History: ${back}`);
    await page.locator("[data-testid=memory-log-row]").nth(1).click();
    await settle(page);
    const change = await seen();
    check(!!change.change, `${name}: one change in it: ${JSON.stringify(change)}`);
    const back2 = await page.locator('[data-testid="nav-back"]:visible').first().getAttribute("title");
    check(/Memory · History/.test(back2 ?? ""), `${name}: Back names History: ${back2}`);

    await step(page, name, -1);
    check(same(await seen(), list), `${name}: Back → the History list: ${JSON.stringify(await seen())}`);
    await step(page, name, -1);
    check(same(await seen(), file), `${name}: Back → the file, History closed: ${JSON.stringify(await seen())}`);
    await step(page, name, 1);
    check(same(await seen(), list), `${name}: Forward → the History list: ${JSON.stringify(await seen())}`);
    await step(page, name, 1);
    check(same(await seen(), change), `${name}: Forward → the same change: ${JSON.stringify(await seen())}`);

    // The pane's own ‹ and Close are steps too, and Back undoes them.
    await page.click("[data-testid=memory-hist-back]");
    await settle(page);
    check(same(await seen(), list), `${name}: the pane's ‹ is the list: ${JSON.stringify(await seen())}`);
    await step(page, name, -1);
    check(same(await seen(), change), `${name}: Back → the change it left: ${JSON.stringify(await seen())}`);
    await page.click("[data-testid=memory-hist-close]");
    await settle(page);
    check(same(await seen(), file), `${name}: Close is the file: ${JSON.stringify(await seen())}`);
    await step(page, name, -1);
    check(same(await seen(), change), `${name}: Back → History, on that change: ${JSON.stringify(await seen())}`);

    // From another file entirely: Back crosses into History on the file before.
    await page.click("[data-testid=memory-hist-close]");
    await settle(page);
    await page.click("[data-testid=memory-file][data-path='MEMORY.md']");
    await settle(page);
    check(same(await seen(), { doc: "MEMORY.md", hist: false, change: null }), `${name}: MEMORY.md, no History: ${JSON.stringify(await seen())}`);
    await step(page, name, -1);
    check(same(await seen(), file), `${name}: Back → nas-layout: ${JSON.stringify(await seen())}`);
    await step(page, name, -1);
    check(same(await seen(), change), `${name}: Back → its History on the change: ${JSON.stringify(await seen())}`);
  } catch (e) {
    check(false, `${name}: ${e.message.split("\n")[0]}`);
  } finally {
    await browser.close();
  }
}

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  await docs(engine, `${name} docs`);
  await memory(engine, `${name} memory`);
}
console.log(failures ? `\n${failures} failure(s)` : "\nall ok");
process.exit(failures ? 1 : 0);
