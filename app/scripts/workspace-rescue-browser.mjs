// Browser gate: a document whose text cannot be saved stays reachable when its beta is
// turned off (Kimi's K2), in Chromium AND WebKit against the mock. An idea with Ideas
// off, a memory file with Workspace chats off, each with the other on and with none.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/workspace-rescue-browser.mjs
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const JELLY = "01JC8Z4K3M9QX2T7VB00000901";
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const isDoc = (page, id) => until(page, (id) => { const d = document.querySelector("[data-testid=main-doc]"); return !!d && !d.hidden && d.checkVisibility() && d.dataset.doc === id; }, id);
const isTab = (page, t) => until(page, (t) => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === t, t, 3000);
const open = (page) => page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();

async function boot(page) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, settings_mode: "advanced", nav_pinned: true, nav_view: "workspaces", beta_ideas: true, beta_workspace_chats: true });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0`);
  await page.waitForSelector("[data-testid=ws-group]");
  const id = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
  await call(page, "workspace_link_idea", { id, idea: JELLY });
  await page.evaluate(() => {
    const t = window.__TAURI_INTERNALS__, real = t.invoke.bind(t);
    window.__fail = false;
    t.invoke = async (c, a) => { if (window.__fail && /^(idea_write|workspace_memory_write)$/.test(c)) throw "the disk is full"; return real(c, a); };
    window.dispatchEvent(new Event("focus"));
  });
  await open(page);
  return id;
}
async function beta(page, key) {
  await page.keyboard.press("Meta+,");
  await page.waitForSelector(".settings-body");
  const box = page.locator(`[data-testid=${key.replace("beta_", "beta-").replace(/_/g, "-")}]`);
  if (!(await box.count())) { await page.fill(".settings-search-input", "beta"); await box.waitFor(); }
  await box.click();
  await page.waitForTimeout(150);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await until(page, () => !document.querySelector(".settings-body"), null, 2000);
}
const KINDS = {
  idea: { tab: "ideas", doc: JELLY, own: "beta_ideas", other: "beta_workspace_chats" },
  memory: { tab: "memory", doc: "MEMORY.md", own: "beta_workspace_chats", other: "beta_ideas" },
};
for (const k of Object.keys(KINDS)) KINDS[k].name = k;

async function stuck(page, k) {
  await page.click(`[data-testid=ws-tab][data-tab=${k.tab}]`);
  if (k.tab === "ideas") await page.locator(`.wsf-ideas [data-idea='${JELLY}'] .wsf-idea-open`).first().click();
  await isDoc(page, k.doc);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.evaluate(() => { window.__fail = true; });
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" STRANDED");
  await until(page, () => !!document.querySelector("[data-testid=ideas-error]"));
}
async function rescue(page, tag, k, both) {
  await boot(page);
  await stuck(page, k);
  await beta(page, k.own);
  if (both) await beta(page, k.other);
  const words = `${tag} ${k.name}${both ? ", both off" : ""}:`;
  ok(await isDoc(page, k.doc) && await isTab(page, k.tab) && /STRANDED/.test(await page.locator("[data-testid=main-doc] .cm-content").innerText()) && !!(await page.$("[data-testid=ideas-error]")), `${words} its beta off, the text is on screen, with why`);
  // Its own tabs are refused while it is not saved; another workspace's Overview says so.
  await page.click("[data-testid=ws-tab][data-tab=overview]");
  ok(await isDoc(page, k.doc), `${words} another tab does not hide it`);
  if (!both) {
    await page.locator("[data-testid=ws-name]", { hasText: "Tools" }).first().click();
    ok(await until(page, () => /in Everything, has text that is not saved/.test(document.querySelector("[data-testid=ws-attention][data-kind=unsaved]")?.textContent ?? "")), `${words} another workspace's Overview says so`);
    await page.click("[data-testid=ws-attention-go]");
    ok(await isDoc(page, k.doc) && await isTab(page, k.tab), `${words} …and Go to it opens it`);
  }
  // From a place, by the workspace's name.
  await page.locator("[data-testid=ws-group] li.row[data-slug]").first().click();
  await until(page, () => !!document.querySelector(".space > .topbar .slug"));
  ok(!both || !(await page.$("[data-testid=place-ws]")), `${words} a place's bar says nothing of a workspace with both off`);
  await open(page);
  ok(await isDoc(page, k.doc), `${words} its workspace's name leads to it`);
  // Saved: its part of the view goes with its beta.
  await page.evaluate(() => { window.__fail = false; });
  await page.locator("[data-testid=ideas-error] button").first().click();
  ok(await until(page, ([tab]) => !document.querySelector("[data-testid=main-doc]") && !document.querySelector(`[data-testid=ws-tab][data-tab=${tab}]`), [k.tab]), `${words} saved, it closes and its tab goes`);
  const text = k.tab === "ideas" ? (await page.evaluate((id) => window.__mock.ideaFile(id).content, JELLY)) : (await call(page, "workspace_memory_file", { id: (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id, path: "MEMORY.md" })).content;
  ok(/STRANDED/.test(text), `${words} …with its text on disk`);
  ok(!both || !(await page.locator("[data-testid=ws-frame]").isVisible().catch(() => false)), `${words} and with both off there is no view of a workspace again`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const k of Object.values(KINDS)) for (const both of [false, true]) {
    if (only && !only.includes(k.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1600, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    const tag = `[${name}]`;
    try { await rescue(page, tag, k, both); } catch (e) { ok(false, `${tag} ${k.name} ${both} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (process.env.IDEAS_SCREENSHOTS) await page.screenshot({ path: `${process.env.IDEAS_SCREENSHOTS}/rs-${name}-${k.name}-${both}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} ${k.name} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nworkspace-rescue-browser: ${failed} failure(s)` : "\nworkspace-rescue-browser: all good");
process.exit(failed ? 1 : 0);
