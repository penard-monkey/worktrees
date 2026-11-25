// Browser gate for leaving a document while it is being saved, in Chromium AND WebKit
// against the mock: what is typed while a write is out reaches the file, or stays on
// screen, whatever was asked for meanwhile (another document, a rename, a restore, a
// new idea, a merge, the Ideas window).
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/documents-leave-browser.mjs
//
// A write is HELD, never slept through: `hold` keeps a row's call until `free`.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const JELLY = "01JC8Z4K3M9QX2T7VB00000901", GPU = "01JC8Z4K3M9QX2T7VB00000902";
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
/** Hold the next calls of `cmd`: before they run, or with their answer in hand. */
const hold = (page, cmd, when = "before") => page.evaluate(([cmd, when]) => {
  const t = window.__TAURI_INTERNALS__;
  if (!window.__held) {
    window.__held = { on: {}, waiting: [], n: 0 };
    const real = t.invoke.bind(t);
    t.invoke = async (c, a) => {
      const w = window.__held.on[c];
      if (!w) return real(c, a);
      window.__held.n++;
      if (w === "before") { await new Promise((r) => window.__held.waiting.push(r)); return real(c, a); }
      const out = await real(c, a);
      await new Promise((r) => window.__held.waiting.push(r));
      return out;
    };
  }
  window.__held.on[cmd] = when;
}, [cmd, when]);
const held = (page, n = 1) => until(page, (n) => window.__held.waiting.length >= n, n);
const free = (page, all = true) => page.evaluate((all) => { if (all) window.__held.on = {}; for (const r of window.__held.waiting.splice(0)) r(); }, all);
const text = (page, id) => page.evaluate((id) => window.__mock.ideaFile(id)?.content ?? null, id);
const isDoc = (page, id) => until(page, (id) => { const d = document.querySelector("[data-testid=main-doc]"); return !!d && !d.hidden && d.dataset.doc === id; }, id);
const editor = (page) => page.locator(".cm-content").first();
async function type(page, s) { await editor(page).click(); await page.keyboard.press("ControlOrMeta+End"); await page.keyboard.type(s); }
const saved = (page) => until(page, () => /Saved/.test(document.querySelector("[data-testid=ideas-save]")?.textContent ?? ""));
/** On disk, or in the editor with something said: never in neither. */
const kept = async (page, id, s) => await until(page, ([id, s]) => (window.__mock.ideaFile(id)?.content ?? "").includes(s), [id, s], 2500)
  || ((await page.locator(".cm-content").first().innerText().catch(() => "")).includes(s) && !!(await page.$("[data-testid=ideas-conflict], [data-testid=ideas-notice], [data-testid=ideas-error], [data-testid=ideas-removed]")));

async function workspace(page) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", beta_ideas: true, beta_workspace_chats: true });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0`);
  await page.waitForSelector("[data-testid=ws-group]");
  const id = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
  for (const i of [JELLY, GPU]) await call(page, "workspace_link_idea", { id, idea: i });
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.click("[data-testid=ws-tab][data-tab=ideas]");
  await page.waitForSelector(`[data-testid=ws-ideas-row][data-idea='${JELLY}'], [data-idea='${JELLY}']`);
  await page.locator(`.wsf-ideas [data-idea='${JELLY}'] .wsf-idea-open, .wsf-ideas [data-idea='${JELLY}']`).first().click();
  await isDoc(page, JELLY);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.waitForSelector("[data-testid=main-doc] .cm-content");
  return id;
}
const toGpu = (page) => page.locator(`.wsf-ideas [data-idea='${GPU}']`).first().click();

async function leave(page, tag) {
  await workspace(page);
  // Typed into every write that leaving starts, one after another: the last one
  // used to be dropped, with the timer that would have saved it.
  await type(page, " W0");
  await hold(page, "idea_write");
  await toGpu(page);
  ok(await held(page), `${tag} leaving starts a save`);
  ok(await page.evaluate((id) => document.querySelector("[data-testid=main-doc]").dataset.doc === id, JELLY), `${tag} …and waits for it`);
  let n = 0;
  while (n < 5 && await held(page) && await page.evaluate(() => !!document.querySelector("[data-testid=main-doc] .cm-content"))) { n++; await type(page, ` W${n}`); await free(page, false); }
  await free(page);
  ok(n === 5, `${tag} what was typed meanwhile is saved too, before anything is closed (${n} rounds)`);
  ok(await isDoc(page, GPU), `${tag} …and then the other one opens`);
  ok(/ W0 W1 W2 W3 W4 W5/.test(await text(page, JELLY)), `${tag} all of it is on disk (${((await text(page, JELLY)).match(/ W\d/g) ?? []).join("")})`);
  // The same through the tab that shows no document, and a place.
  await page.locator(`.wsf-ideas [data-idea='${JELLY}']`).first().click();
  await isDoc(page, JELLY);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]").catch(() => {});
  await hold(page, "idea_write");
  await type(page, " FOUR");
  await page.click("[data-testid=ws-tab][data-tab=memory]");
  await held(page);
  await type(page, " FIVE");
  await free(page);
  ok(await until(page, (id) => / FOUR FIVE/.test(window.__mock.ideaFile(id).content), JELLY), `${tag} another tab: on disk (${JSON.stringify([(await text(page, JELLY)).slice(-40), (await text(page, GPU)).slice(-40)])})`);
  // A refusal of the SECOND save keeps the document.
  await page.click("[data-testid=ws-tab][data-tab=ideas]");
  await isDoc(page, JELLY);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]").catch(() => {});
  await hold(page, "idea_write");
  await type(page, " SIX");
  await toGpu(page);
  await held(page);
  await type(page, " SEVEN");
  await page.evaluate(() => { window.__held.then = true; });
  await free(page, false);
  await held(page);
  await page.evaluate(() => window.__mock.ideaFailWrites("the disk is full"));
  await free(page);
  ok(await until(page, () => !!document.querySelector("[data-testid=ideas-error]")) && (await editor(page).innerText()).includes(" SEVEN") && await isDoc(page, JELLY), `${tag} a second save that fails keeps the document, and says so`);
}

async function acts(page, tag) {
  await workspace(page);
  const title = async () => (await page.locator("[data-testid=main-doc] .ix-title").innerText());
  // ── rename: typed while its save, then while the rename itself, is out ──
  const rename = async (to) => {
    await page.click("[data-testid=main-doc] [data-testid=doc-menu-open]");
    await page.click("[data-testid=ideas-rename]");
    await page.fill("[data-testid=ideas-rename-input]", to);
    await page.press("[data-testid=ideas-rename-input]", "Enter");
  };
  await type(page, " R1");
  await hold(page, "idea_write");
  await rename("Renamed once");
  await held(page);
  await type(page, " R2");
  await free(page);
  ok(await kept(page, JELLY, " R1 R2") && await until(page, () => /Renamed once/.test(document.querySelector("[data-testid=main-doc] .ix-title").textContent)), `${tag} rename: typed during its save (${await title()})`);
  await saved(page);
  await hold(page, "idea_rename", "after");
  await rename("Renamed twice");
  await held(page);
  await type(page, " R3");
  await free(page);
  ok(await kept(page, JELLY, " R3"), `${tag} rename: typed during the rename`);
  if (await page.$("[data-testid=ideas-keep-mine]")) await page.click("[data-testid=ideas-keep-mine]");
  ok(await until(page, (id) => window.__mock.ideaFile(id).content.includes(" R3"), JELLY), `${tag} …and Keep mine saves it`);
  // ── restore: typed while it is out ──
  await saved(page);
  { const d = await call(page, "idea_read", { id: JELLY }); await call(page, "idea_write", { id: JELLY, content: `${d.content}\nA version.\n`, expectedMtime: d.mtime_ms, keepHistory: true }); }
  await until(page, () => /A version\./.test(document.querySelector("[data-testid=main-doc] .cm-content")?.innerText ?? ""));
  if (!(await page.$("[data-testid=ideas-versions]"))) await page.click("[data-testid=ideas-versions-toggle]");
  await page.waitForSelector("[data-testid=ideas-version-restore]");
  await hold(page, "idea_restore", "after");
  await page.locator("[data-testid=ideas-version-restore]").first().click();
  await page.locator("[data-testid=ideas-version-restore]").first().click();
  await held(page);
  await type(page, " V1");
  await free(page);
  ok(await kept(page, JELLY, " V1"), `${tag} restore: what was typed meanwhile is kept`);
}


async function companion(page, tag) {
  await page.goto(`${BASE}/companion/index.html?ideas=seed`);
  await page.waitForSelector("[data-testid=ideas-rows] li");
  await page.locator(`.ix-row[data-idea='${JELLY}'] button`).first().click();
  await page.waitForSelector(".cm-content");
  // ── a new idea: typed into the one before while it is made ──
  await hold(page, "idea_create", "after");
  await page.click("[data-testid=ideas-new]");
  await held(page);
  await type(page, " N1");
  await free(page);
  ok(await kept(page, JELLY, " N1"), `${tag} a new idea: what was typed meanwhile is saved`);
  // ── another row, held ──
  await page.locator(`.ix-row[data-idea='${JELLY}'] button`).first().click();
  await until(page, (id) => document.querySelector(".ix-row.on, .ix-row-on")?.dataset.idea === id, JELLY, 1500);
  await page.waitForTimeout(200);
  await type(page, " C1");
  await hold(page, "idea_write");
  await page.locator(`.ix-row[data-idea='${GPU}'] button`).first().click();
  await held(page);
  await type(page, " C2");
  await free(page);
  ok(await until(page, (id) => / C1 C2/.test(window.__mock.ideaFile(id).content), JELLY), `${tag} another row: all of it is on disk`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [leave, acts, companion]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (process.env.IDEAS_SCREENSHOTS) await page.screenshot({ path: `${process.env.IDEAS_SCREENSHOTS}/dl-${name}-${part.name}-FAIL.png` }).catch(() => {}); }
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\ndocuments-leave-browser: ${failed} failure(s)` : "\ndocuments-leave-browser: all good");
process.exit(failed ? 1 : 0);
