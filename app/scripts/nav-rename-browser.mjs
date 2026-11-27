// Browser gate: renaming a lane and a workspace chat in the nav (rowname.ts,
// RowNameField.tsx), in Chromium AND WebKit against the mock. Real pointer
// and key input: a synthetic event skips hit-testing (AGENTS.md).
//
//   VITE_MOCK=1 vite --port 1471 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     NAV_RENAME_URL=http://localhost:1471 node app/scripts/nav-rename-browser.mjs
//
// Shot dir: NAV_RENAME_SHOTS=<dir> saves a lane row and a chat row mid-edit.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.NAV_RENAME_URL ?? "http://localhost:1471";
const SHOTS = process.env.NAV_RENAME_SHOTS;
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 4000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);

const SLUG = "messaging";
const ROW = `li.row:not(.ghost)[data-slug="${SLUG}"]`;
const FIELD = "[data-testid=row-name-input]";
const rowName = (page) => page.locator(`${ROW} .row-name`).first().textContent().then((t) => (t ?? "").replace(/^[◆★] /, "").trim(), () => null);
const topName = (page) => page.locator(".topbar .slug").first().textContent().then((t) => (t ?? "").trim(), () => null);

async function boot(browser, view) {
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 900 } })).newPage();
  page.setDefaultTimeout(10_000);
  const errors = [];
  // xterm in a headless browser throws on `dimensions` when an entered
  // place's terminal is torn down; every browser gate here filters it.
  page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: view, beta_workspace_chats: true, beta_ideas: true });
  await page.goto(`${BASE}/?agentprompt=off&ideas=seed&workspaces=1&folding=0`);
  return { page, errors };
}

/** The field is up, focused, and holds `name` all selected. */
const fieldReady = (page, name) => until(page, (name) => {
  const f = document.activeElement;
  return f?.matches?.("[data-testid=row-name-input], [data-testid=ws-chat-row-name-input]") && f.value === name && f.selectionStart === 0 && f.selectionEnd === name.length;
}, name);

async function lanes(browser, tag) {
  const { page, errors } = await boot(browser, "projects");
  const row = page.locator(ROW).first();
  await row.waitFor();
  await row.scrollIntoViewIfNeeded();

  // A single click selects, at once: no delay waits for a second click.
  await row.click();
  ok(await row.evaluate((e) => e.classList.contains("sel")), `${tag} a click selects the row at once`);
  ok(await page.locator(FIELD).count() === 0, `${tag} …and starts no rename`);

  // Right-click → Rename.
  await row.click({ button: "right" });
  await page.locator("[data-testid=ctx-rename]").click();
  ok(await fieldReady(page, SLUG), `${tag} Rename… in the row's menu opens its field on the name, selected`);
  if (SHOTS && tag === "chromium") {
    await page.keyboard.type("Release prep");
    const nav = await page.locator(ROW).first().boundingBox();
    await page.screenshot({ path: `${SHOTS}/nav-rename-lane.png`, clip: { x: 0, y: Math.max(0, nav.y - 90), width: 340, height: 200 } });
    await page.keyboard.press("ControlOrMeta+a");
  }
  await page.keyboard.type("Release prep");
  await page.keyboard.press("Enter");
  ok(await until(page, () => !document.querySelector("[data-testid=row-name-input]")), `${tag} Enter closes the field`);
  ok(await rowName(page) === "Release prep", `${tag} …the row shows the new name (${await rowName(page)})`);
  ok(await topName(page) === "Release prep", `${tag} …and so does the header (${await topName(page)})`);
  ok(await page.locator(ROW).count() > 0, `${tag} …and the slug is unchanged`);

  // F2 renames the selected lane. Empty returns it to its slug.
  await page.locator("body").click({ position: { x: 700, y: 10 } }).catch(() => {});
  await page.evaluate(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await page.keyboard.press("F2");
  ok(await fieldReady(page, "Release prep"), `${tag} F2 opens the selected lane's field`);
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Enter");
  ok(await until(page, (s) => document.querySelector(`li.row:not(.ghost)[data-slug="${s}"] .row-name`)?.textContent?.trim().endsWith(s), SLUG), `${tag} an empty name returns the lane to its slug (${await rowName(page)})`);

  // Escape cancels.
  await page.keyboard.press("F2");
  await fieldReady(page, SLUG);
  await page.keyboard.type("Not this");
  await page.keyboard.press("Escape");
  ok(await until(page, () => !document.querySelector("[data-testid=row-name-input]")), `${tag} Escape closes the field`);
  ok(await rowName(page) === SLUG, `${tag} …and keeps the name (${await rowName(page)})`);
  ok(await page.locator(ROW).first().evaluate((e) => e.classList.contains("sel")), `${tag} …and the place stays selected`);

  // Blur saves.
  await page.keyboard.press("F2");
  await fieldReady(page, SLUG);
  await page.keyboard.type("Blurred");
  await page.locator(".topbar").first().click({ position: { x: 5, y: 5 } });
  ok(await until(page, () => !document.querySelector("[data-testid=row-name-input]")), `${tag} a click elsewhere closes the field`);
  ok(await until(page, () => /Blurred/.test(document.querySelector('li.row:not(.ghost)[data-slug="messaging"] .row-name')?.textContent ?? "")), `${tag} …and saves it (${await rowName(page)})`);

  // A double-click on the row still enters the place, and opens no field.
  const r2 = page.locator(ROW).first();
  await r2.dblclick();
  await page.waitForTimeout(200);
  ok(await page.locator(FIELD).count() === 0, `${tag} a double-click on the row opens no field (it enters, as before)`);

  // A press inside the field arms no drag; on the row it still does.
  await page.keyboard.press("F2");
  await fieldReady(page, "Blurred");
  const fb = await page.locator(FIELD).boundingBox();
  await page.mouse.move(fb.x + 10, fb.y + fb.height / 2);
  await page.mouse.down();
  await page.mouse.move(fb.x + 40, fb.y + 60, { steps: 6 });
  ok(!(await page.evaluate(() => document.body.classList.contains("dragging") || document.body.classList.contains("dragging-place"))), `${tag} a press in the field starts no drag`);
  await page.mouse.up();
  await page.keyboard.press("Escape");
  await until(page, () => !document.querySelector("[data-testid=row-name-input]"));
  const rb = await page.locator(ROW).first().boundingBox();
  await page.mouse.move(rb.x + rb.width / 2, rb.y + rb.height / 2);
  await page.mouse.down();
  await page.mouse.move(rb.x + rb.width / 2 + 12, rb.y + rb.height / 2 + 30, { steps: 6 });
  ok(await until(page, () => document.body.classList.contains("dragging") || document.body.classList.contains("dragging-place"), undefined, 1500), `${tag} a press on the row still starts a drag`);
  await page.keyboard.press("Escape");
  await page.mouse.up();

  ok(errors.length === 0, `${tag} no page errors ${errors.join(" | ")}`);
  await page.context().close();
}

async function chats(browser, tag) {
  const { page, errors } = await boot(browser, "workspaces");
  await page.waitForSelector("[data-testid=ws-chat-row]");
  const row = () => page.locator("[data-testid=ws-chat-row]").first();
  const conv = await row().getAttribute("data-chat");
  const title = ((await row().locator(".wsc-title").textContent()) ?? "").trim();
  const sel = `[data-testid=ws-chat-row][data-chat="${conv}"]`;
  const shown = () => page.locator(`${sel} .wsc-title`).textContent().then((t) => (t ?? "").trim(), () => null);

  // A click opens the chat, and starts no rename.
  await row().click();
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-col]")), `${tag} a click on a chat row opens the chat`);
  ok(await page.locator("[data-testid=ws-chat-row-name-input]").count() === 0, `${tag} …and starts no rename`);

  // Right-click → Rename.
  await page.locator(sel).click({ button: "right" });
  await page.locator("[data-testid=ws-chat-row-rename]").click();
  ok(await fieldReady(page, title), `${tag} Rename… in a chat row's menu opens its field on the title, selected`);
  if (SHOTS && tag === "chromium") {
    await page.keyboard.type("Order of release");
    const b = await page.locator(sel).boundingBox();
    await page.screenshot({ path: `${SHOTS}/nav-rename-chat.png`, clip: { x: 0, y: Math.max(0, b.y - 90), width: 340, height: 200 } });
    await page.keyboard.press("ControlOrMeta+a");
  }
  await page.keyboard.type("Order of release");
  await page.keyboard.press("Enter");
  ok(await until(page, (s) => document.querySelector(`${s} .wsc-title`)?.textContent?.trim() === "Order of release", sel), `${tag} Enter saves the chat's title (${await shown()})`);
  ok(await until(page, () => document.querySelector("[data-testid=ws-chat-col] .wsc-name-t")?.textContent?.trim() === "Order of release"), `${tag} …and the chat's header says it`);
  ok(await page.locator(sel).count() === 1, `${tag} …and the chat keeps its id`);

  // F2 renames the chat on screen; Escape cancels; an empty title is a cancel.
  await page.evaluate(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await page.keyboard.press("F2");
  ok(await fieldReady(page, "Order of release"), `${tag} F2 opens the open chat's row field`);
  await page.keyboard.type("Not this");
  await page.keyboard.press("Escape");
  ok(await until(page, () => !document.querySelector("[data-testid=ws-chat-row-name-input]")), `${tag} Escape closes it`);
  ok(await shown() === "Order of release", `${tag} …and keeps the title`);
  ok(await page.locator("[data-testid=ws-chat-col]").count() === 1, `${tag} …and the chat stays open`);
  await page.keyboard.press("F2");
  await fieldReady(page, "Order of release");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(200);
  ok(await shown() === "Order of release", `${tag} an empty title is a cancel, never an error`);

  // Blur saves.
  await page.keyboard.press("F2");
  await fieldReady(page, "Order of release");
  await page.keyboard.type("Blurred chat");
  await page.locator("[data-testid=ws-chat-col]").click({ position: { x: 200, y: 200 } });
  ok(await until(page, (s) => document.querySelector(`${s} .wsc-title`)?.textContent?.trim() === "Blurred chat", sel), `${tag} a click elsewhere saves the chat's title (${await shown()})`);

  // The header's own rename saves on blur too, now.
  await page.locator("[data-testid=ws-chat-menu]").click();
  await page.locator("[data-testid=ws-chat-rename]").click();
  ok(await until(page, () => document.activeElement?.dataset?.testid === "ws-chat-rename-input"), `${tag} the header's Rename… still opens its field`);
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("From the header");
  await page.locator(sel).click({ position: { x: 4, y: 4 } }).catch(() => {});
  ok(await until(page, (s) => document.querySelector(`${s} .wsc-title`)?.textContent?.trim() === "From the header", sel), `${tag} …and blur saves it there too (${await shown()})`);

  ok(errors.length === 0, `${tag} no page errors ${errors.join(" | ")}`);
  await page.context().close();
}

for (const [tag, type] of [["chromium", chromium], ["webkit", webkit]]) {
  const browser = await type.launch();
  try { await lanes(browser, tag); await chats(browser, tag); }
  catch (e) { failed++; console.log(`FAIL ${tag} threw: ${e.message.split("\n")[0]}`); }
  finally { await browser.close(); }
}
console.log(failed ? `${failed} failed` : "all ok");
process.exit(failed ? 1 : 0);
