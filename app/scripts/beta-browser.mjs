// Optional browser gate for the beta features (app/src/beta.ts), in Chromium
// AND WebKit, against the mock: what the main window DRAWS with each flag off
// and on, and that flipping one needs no reload.
// (beta-check.mjs holds the same rules on the source.)
//
//   VITE_MOCK=1 vite --port 1468 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     BETA_MOCK_URL=http://localhost:1468 node app/scripts/beta-browser.mjs
//
// What it holds:
//   - Named workspaces are NOT a beta (they left it on 2026-11-14): with every
//     flag off, a fresh tab with workspaces seeded and `nav_view:
//     "workspaces"` stored opens on the Workspaces view, the switch is there,
//     and a project's menu has Add to workspace and says "Remove from All
//     projects". Turning Ideas on and off again changes none of that.
//   - Ideas is a beta of its own: off, no lightbulb and ⌘⇧I opens nothing;
//     ticked in the sheet, both appear at once with no reload; off again, the
//     Ideas tab `open_companion` opened is closed and both are gone.
//   - Settings: Beta features is not drawn in Beginner, is in Advanced, the
//     header's search finds it by "beta", and it draws ONE checkbox per beta
//     feature (beta.ts BETA_FEATURES): "Ideas" and "Group chat"
//     (groupchat-beta-browser.mjs drives the second).
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

/** Workspaces, whatever any beta says. Leaves the nav on Workspaces. */
async function workspaces(page, tag, when) {
  ok(await has(page, "[data-testid=nav-view-workspaces]") && await has(page, "[data-testid=nav-view-projects]"), `${tag} ${when}: the Workspaces | All projects switch is there`);
  if (!(await has(page, "[data-testid=nav-view-workspaces][aria-selected=true]"))) {
    await page.click("[data-testid=nav-view-workspaces]");
    await page.waitForTimeout(150);
  }
  ok(await has(page, "[data-testid=ws-group]"), `${tag} ${when}: the Workspaces view draws its cards`);
  await page.click("[data-testid=nav-view-projects]");
  await page.waitForTimeout(150);
  await ctxOnFirstProject(page);
  const t = await menuText(page);
  ok(await has(page, "[data-testid=ctx-ws-add]"), `${tag} ${when}: a project's menu has Add to workspace`);
  // In All projects a project is under no workspace, so its menu has no
  // "Remove from workspace": that wording now means a named one only.
  ok(t.includes("Remove from All projects") && !t.includes("Remove from workspace"), `${tag} ${when}: …and says "Remove from All projects"`);
  await closeMenu(page);
  await page.click("[data-testid=nav-view-workspaces]");
  await page.waitForTimeout(150);
}

async function ideasOff(page, tag, when) {
  ok(!(await has(page, "[data-testid=ideas-rail]")), `${tag} ${when}: no lightbulb in the rail`);
  ok((await chord(page)) === null, `${tag} ${when}: ⌘⇧I opens nothing`);
}

async function run(engine, name) {
  const browser = await engine.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 1400 } });
  const page = await ctx.newPage();
  // Workspaces seeded, the Workspaces view stored as the nav's last view, and
  // NO beta key: every flag off.
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "left", settings_rev: 5, settings_mode: "beginner", nav_pinned: true, nav_view: "workspaces" });
  await page.goto(`${BASE}/?workspaces=1`);
  await page.waitForSelector("[data-track=settings]");
  await page.waitForSelector("[data-testid=nav-view-workspaces]", { timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(300);
  const tag = `[${name}]`;

  ok(await has(page, "[data-testid=nav-view-workspaces][aria-selected=true]"), `${tag} every flag off: the stored Workspaces view opens`);
  await workspaces(page, tag, "every flag off");
  await ideasOff(page, tag, "Ideas off");

  // Settings: Advanced and Expert only, findable by search, one checkbox each.
  await openSheet(page);
  await toGeneral(page);
  ok(!(await has(page, ".settings-body [data-sec=beta]")), `${tag} Beginner: Beta features is not drawn`);
  await setMode(page, "advanced");
  ok(await has(page, ".settings-body [data-sec=beta]"), `${tag} Advanced: Beta features is drawn on General`);
  const boxes = await page.$$eval(".settings-body [data-sec=beta] input[type=checkbox]", (b) => b.map((x) => x.closest("label")?.innerText.trim()));
  ok(JSON.stringify(boxes) === JSON.stringify(["Ideas", "Group chat", "Workspace chats"]), `${tag} …one checkbox per beta feature, Ideas, Group chat and Workspace chats: ${JSON.stringify(boxes)}`);
  ok(!(await page.evaluate(() => document.querySelector(".settings-body [data-sec=beta]")?.innerText.includes("workspaces"))), `${tag} …and it no longer mentions workspaces`);
  await page.fill(".settings-search-input", "beta");
  await page.waitForTimeout(200);
  ok(await page.evaluate(() => document.querySelector(".settings-body")?.innerText.includes("Beta features") ?? false), `${tag} the header's search finds it by "beta"`);
  await page.fill(".settings-search-input", "");
  await page.waitForTimeout(150);
  await toGeneral(page);

  // Ideas ON, with no reload.
  await page.click("[data-testid=beta-ideas]");
  // The blob is saved after a debounce: wait for it rather than guess.
  const wrote = await page.waitForFunction(() => JSON.parse(sessionStorage.getItem("wt-mock-ui-state") ?? "null")?.beta_ideas === true, null, { timeout: 3000 }).then(() => true, () => false);
  ok(wrote, `${tag} the Ideas checkbox writes beta_ideas: true`);
  await closeSheet(page);
  await page.waitForSelector("[data-testid=ideas-rail]", { timeout: 3000 }).catch(() => {});
  ok(await has(page, "[data-testid=ideas-rail]"), `${tag} Ideas on: the lightbulb appears at once`);
  await workspaces(page, tag, "Ideas on");
  const ideas = await chord(page);
  ok(ideas !== null, `${tag} Ideas on: ⌘⇧I opens the Ideas tab`);
  await page.bringToFront();

  // Ideas OFF again: its tab closes, it goes; workspaces stay.
  await openSheet(page);
  await setMode(page, "advanced");
  await toGeneral(page);
  await page.click("[data-testid=beta-ideas]");
  await page.waitForTimeout(300);
  ok(ideas === null || ideas.isClosed(), `${tag} Ideas off again: the Ideas tab is closed`);
  await closeSheet(page);
  await ideasOff(page, tag, "Ideas off again");
  await workspaces(page, tag, "Ideas off again");

  await browser.close();
}

for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  try { await run(engine, name); } catch (e) { failed++; console.log(`FAIL [${name}] ${e.stack ?? e}`); }
}
if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("\nbeta-browser: all ok");
