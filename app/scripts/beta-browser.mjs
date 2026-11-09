// Optional browser gate for the Ideas and workspaces beta (app/src/beta.ts),
// in Chromium AND WebKit, against the mock: what the main window DRAWS with
// the flag off and on, and that flipping it needs no reload.
// (beta-check.mjs holds the same rules on the source.)
//
//   VITE_MOCK=1 vite --port 1468 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     BETA_MOCK_URL=http://localhost:1468 node app/scripts/beta-browser.mjs
//
// What it holds:
//   - OFF (a fresh tab, the mock's default, with workspaces seeded and
//     `nav_view: "workspaces"` stored): no lightbulb, no nav switch, the nav is
//     All projects, "All projects" is nowhere on screen, a project's menu has
//     no workspace item and says "Remove from workspace", ⌘⇧I opens nothing,
//     and `nav_view` is still "workspaces" in the blob;
//   - Settings: Beta features is not drawn in Beginner, is in Advanced, and
//     the header's search finds it by "beta";
//   - ON, ticked in the sheet: the lightbulb and the switch appear at once
//     (Workspaces, as `nav_view` held), the menu has Add to workspace and
//     "Remove from All projects", ⌘⇧I opens the Ideas tab;
//   - OFF again: the Ideas tab `open_companion` opened is closed, and
//     everything above is gone again.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.BETA_MOCK_URL ?? "http://localhost:1468";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const has = async (page, sel) => (await page.$$(sel)).length > 0;
const ctxOnFirstProject = async (page) => {
  await page.evaluate(() => {
    const el = document.querySelector(".nav-scroll .project[data-project-root] .project-h");
    const r = el.getBoundingClientRect();
    el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 30, clientY: r.top + 5 }));
  });
  await page.waitForTimeout(100);
};
const menuText = (page) => page.evaluate(() => document.querySelector(".ctxmenu")?.innerText ?? "");
const closeMenu = async (page) => { await page.keyboard.press("Escape"); await page.waitForTimeout(80); };
const blob = (page) => page.evaluate(() => JSON.parse(sessionStorage.getItem("wt-mock-ui-state") ?? "null"));
const openSheet = async (page) => { await page.click("[data-track=settings]"); await page.waitForSelector(".settings-body .page-title"); };
const closeSheet = async (page) => { await page.keyboard.press("Escape"); await page.waitForTimeout(150); };
const setMode = async (page, m) => { await page.click(`[data-testid=settings-mode] [data-mode=${m}]`); await page.waitForTimeout(150); };
const toGeneral = async (page) => { await page.click(".settings-cat[data-cat=general]"); await page.waitForSelector(".settings-body[data-page=general]"); await page.waitForTimeout(150); };
/** ⌘⇧I, and whether a popup (the mock's Ideas tab) opened within a beat. */
const chord = async (page) => {
  const popup = page.waitForEvent("popup", { timeout: 800 }).catch(() => null);
  await page.keyboard.press("Meta+Shift+KeyI");
  return popup;
};

async function off(page, tag, when) {
  ok(!(await has(page, "[data-testid=ideas-rail]")), `${tag} ${when}: no lightbulb in the rail`);
  ok(!(await has(page, "[data-testid=nav-view-projects]")), `${tag} ${when}: no All projects | Workspaces switch`);
  ok(!(await has(page, "[data-testid=ws-group]")) && (await page.$$(".nav-scroll li.row[data-slug]")).length > 0, `${tag} ${when}: the nav is All projects (rows, no workspace groups)`);
  ok(!(await page.evaluate(() => document.body.innerText.includes("All projects"))), `${tag} ${when}: "All projects" is nowhere on screen`);
  await ctxOnFirstProject(page);
  const t = await menuText(page);
  ok(!(await has(page, "[data-testid=ctx-ws-add]")) && !/workspace ›|Add to workspace/.test(t), `${tag} ${when}: a project's menu has no Add to workspace`);
  ok(t.includes("Remove from workspace") && !t.includes("Remove from All projects"), `${tag} ${when}: …and says "Remove from workspace"`);
  await closeMenu(page);
  ok((await chord(page)) === null, `${tag} ${when}: ⌘⇧I opens nothing`);
}

async function run(engine, name) {
  const browser = await engine.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 1400 } });
  const page = await ctx.newPage();
  // Workspaces seeded, the Workspaces view stored as the nav's last view, and
  // NO beta key: an install that had the view open in a build without the flag.
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "left", settings_rev: 5, settings_mode: "beginner", nav_pinned: true, nav_view: "workspaces" });
  await page.goto(`${BASE}/?workspaces=1`);
  await page.waitForSelector("[data-track=settings]");
  await page.waitForSelector(".nav-scroll li.row[data-slug]");
  await page.waitForTimeout(300);
  const tag = `[${name}]`;

  await off(page, tag, "off by default");
  ok((await blob(page))?.nav_view === "workspaces", `${tag} off: nav_view is kept as stored`);

  // Settings: Advanced and Expert only, findable by search.
  await openSheet(page);
  await toGeneral(page);
  ok(!(await has(page, ".settings-body [data-sec=beta]")), `${tag} Beginner: Beta features is not drawn`);
  await setMode(page, "advanced");
  ok(await has(page, ".settings-body [data-sec=beta]"), `${tag} Advanced: Beta features is drawn on General`);
  await page.fill(".settings-search-input", "beta");
  await page.waitForTimeout(200);
  ok(await page.evaluate(() => document.querySelector(".settings-body")?.innerText.includes("Beta features") ?? false), `${tag} the header's search finds it by "beta"`);
  await page.fill(".settings-search-input", "");
  await page.waitForTimeout(150);
  await toGeneral(page);

  // ON, with no reload.
  await page.click("[data-testid=beta-ideas]");
  // The blob is saved after a debounce: wait for it rather than guess.
  const wrote = await page.waitForFunction(() => JSON.parse(sessionStorage.getItem("wt-mock-ui-state") ?? "null")?.beta_ideas === true, null, { timeout: 3000 }).then(() => true, () => false);
  ok(wrote, `${tag} the switch writes beta_ideas: true`);
  await closeSheet(page);
  await page.waitForSelector("[data-testid=nav-view-projects]", { timeout: 3000 }).catch(() => {});
  ok(await has(page, "[data-testid=ideas-rail]"), `${tag} on: the lightbulb appears at once`);
  ok(await has(page, "[data-testid=nav-view-workspaces][aria-selected=true]"), `${tag} on: the switch appears, back on Workspaces (nav_view kept)`);
  ok(await has(page, "[data-testid=ws-group]"), `${tag} on: the Workspaces view draws its groups`);
  await page.click("[data-testid=nav-view-projects]");
  await page.waitForTimeout(150);
  await ctxOnFirstProject(page);
  const t = await menuText(page);
  ok(await has(page, "[data-testid=ctx-ws-add]"), `${tag} on: a project's menu has Add to workspace`);
  ok(t.includes("Remove from All projects"), `${tag} on: …and says "Remove from All projects"`);
  await closeMenu(page);
  const ideas = await chord(page);
  ok(ideas !== null, `${tag} on: ⌘⇧I opens the Ideas tab`);
  await page.bringToFront();

  // OFF again: the Ideas tab closes, everything goes.
  await openSheet(page);
  await setMode(page, "advanced");
  await toGeneral(page);
  await page.click("[data-testid=beta-ideas]");
  await page.waitForTimeout(300);
  ok(ideas === null || ideas.isClosed(), `${tag} off again: the Ideas tab is closed`);
  await closeSheet(page);
  await off(page, tag, "off again");

  await browser.close();
}

for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  try { await run(engine, name); } catch (e) { failed++; console.log(`FAIL [${name}] ${e.stack ?? e}`); }
}
if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("\nbeta-browser: all ok");
