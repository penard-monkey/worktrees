// Browser gate for Float and Dock (app/src/doc/handoff.ts, MainDocument's float,
// app/companion/MemoryDoc.tsx), in Chromium AND WebKit against the mock: a document
// stays where it is until the other window says it has it open, and every way that
// does not happen is said there.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/documents-float-browser.mjs
//
// Each tab has a mock of its own (the same seed), so what one writes the other does
// not read: the hand-off is what is tested here, not a shared disk.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const JELLY = "01JC8Z4K3M9QX2T7VB00000901", GPU = "01JC8Z4K3M9QX2T7VB00000902";
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 6000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const isDoc = (page, id) => until(page, (id) => { const d = document.querySelector("[data-testid=main-doc]"); return !!d && !d.hidden && d.dataset.doc === id; }, id);
const says = (page, re) => until(page, (re) => new RegExp(re).test(document.querySelector("[data-testid=main-doc] [data-testid=ideas-notice], .notice, [data-testid=notice]")?.textContent ?? "") || new RegExp(re).test(document.body.innerText), re.source, 4000);
const floatBtn = (page) => page.locator("[data-testid=main-doc] [data-testid=main-doc-float]");
const card = (page) => until(page, () => !!document.querySelector("[data-testid=ws-floated]"), null, 4000);
const mems = (page) => page.evaluate(() => document.querySelector("[data-testid=main-doc]")?.dataset.doc ?? null);

async function main(page, betas = ["beta_ideas", "beta_workspace_chats"]) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", ...Object.fromEntries(betas.map((b) => [b, true])) });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0`);
  await page.waitForSelector("[data-testid=ws-group]");
  const id = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
  if (betas.includes("beta_ideas")) for (const i of [JELLY, GPU]) await call(page, "workspace_link_idea", { id, idea: i });
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-tab]");
  return id;
}
async function idea(page, id = JELLY) {
  await page.click("[data-testid=ws-tab][data-tab=ideas]");
  await page.locator(`.wsf-ideas [data-idea='${id}']`).first().click();
  await isDoc(page, id);
}
async function memory(page, path = "MEMORY.md") {
  await page.click("[data-testid=ws-tab][data-tab=memory]");
  await isDoc(page, "MEMORY.md");
  if (path !== "MEMORY.md") { await page.locator(`[data-testid=memory-file][data-path='${path}']`).first().click(); await isDoc(page, path); }
}
/** Nobody opens, or it is open already (the harness's open RELOADS a tab it has;
 *  the app raises the window): the window's door does nothing (or throws). */
const noWindow = (page, why = null) => page.evaluate((why) => { const t = window.__TAURI_INTERNALS__, i = t.invoke; t.invoke = (c, a) => (c === "open_companion" ? (why ? Promise.reject(why) : Promise.resolve(null)) : i(c, a)); }, why);

async function waits(page, tag) {
  await main(page);
  await idea(page);
  await noWindow(page);
  await floatBtn(page).click();
  ok(await until(page, () => !!localStorage.getItem("wt.ideas.show")), `${tag} Float asks the Ideas window`);
  await page.waitForTimeout(500);
  ok(await isDoc(page, JELLY) && await floatBtn(page).isDisabled() && !(await page.$("[data-testid=ws-floated]")), `${tag} …and the document stays until it answers`);
  // Typed while it is asked: saved, then closed.
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" WHILE ASKED");
  await page.evaluate(() => { const a = JSON.parse(localStorage.getItem("wt.ideas.show")); localStorage.removeItem("wt.ideas.show"); localStorage.setItem("wt.ideas.shown", JSON.stringify({ ask: a.ask, at: Date.now(), ok: true, doc: a.doc })); });
  ok(await card(page) && !(await mems(page)), `${tag} answered: it moved, and the card is in its place`);
  ok((await page.evaluate((id) => window.__mock.ideaFile(id).content, JELLY)).includes(" WHILE ASKED"), `${tag} …with what was typed meanwhile on disk`);
  // ── the answer names another document ──
  await page.click("[data-testid=ws-floated-here]");
  await isDoc(page, JELLY);
  await floatBtn(page).click();
  await until(page, () => !!localStorage.getItem("wt.ideas.show"));
  await page.evaluate((gpu) => { const a = JSON.parse(localStorage.getItem("wt.ideas.show")); localStorage.setItem("wt.ideas.shown", JSON.stringify({ ask: a.ask, at: Date.now(), ok: true, doc: { kind: "idea", id: gpu } })); }, GPU);
  ok(await says(page, /opened another document instead\. “.*” is still here\./) && await isDoc(page, JELLY), `${tag} another document open there is not it: it stays, and says so`);
}

async function fails(page, tag) {
  await main(page);
  await idea(page);
  await noWindow(page, "no such window");
  await floatBtn(page).click();
  ok(await says(page, /The Ideas window did not open: no such window/) && await isDoc(page, JELLY) && await floatBtn(page).isEnabled(), `${tag} a window that does not open: it stays, and says so`);
  ok(await page.evaluate(() => !localStorage.getItem("wt.ideas.show")), `${tag} …and the ask is withdrawn`);
  await page.evaluate(() => { window.__set = Storage.prototype.setItem; Storage.prototype.setItem = function () { throw new Error("QuotaExceededError"); }; });
  await floatBtn(page).click();
  ok(await says(page, /storage is not available/) && await isDoc(page, JELLY), `${tag} no storage: the same`);
  await page.evaluate(() => { Storage.prototype.setItem = window.__set; });
  // The dock's Float goes the same way: nothing is claimed.
  await page.locator("[data-testid=ws-group] li.row[data-slug]").nth(1).click();
  if (await page.locator("[data-testid=place-ws]").textContent().then((t) => /workspaces/.test(t))) { await page.click("[data-testid=place-ws]"); await page.locator("[data-testid=place-ws-pick]").first().click(); }
  await page.locator("[data-track='dock.workspace']").first().click();
  await page.click("[data-testid=dock-ws-tab][data-tab=ideas]");
  if (!(await page.$("[data-testid=dock-ws-float]"))) { await page.click("[data-testid=dock-ws-ideas]"); await page.click(`[data-testid=dock-ws-idea][data-idea='${GPU}']`); }
  await page.click("[data-testid=dock-ws-float]");
  ok(await says(page, /The Ideas window did not open: no such window/), `${tag} a place's dock: the same`);
}

async function silent(page, tag) {
  await main(page);
  await idea(page);
  await noWindow(page);
  await page.clock.install();
  await floatBtn(page).click();
  await until(page, () => !!localStorage.getItem("wt.ideas.show"));
  await page.clock.runFor(13_000);
  ok(await says(page, /The Ideas window did not open it\./) && await isDoc(page, JELLY) && await page.evaluate(() => !localStorage.getItem("wt.ideas.show")), `${tag} nobody answers: it stays, says so, and the ask is withdrawn`);
}

async function ideas(page, tag) {
  const ctx = page.context();
  await main(page);
  await idea(page);
  const [w] = await Promise.all([ctx.waitForEvent("page"), floatBtn(page).click()]);
  ok(await until(w, () => /Jellyfin/.test(document.querySelector(".ix-title")?.textContent ?? ""), null, 10000) && await card(page), `${tag} floated: open there, and the card here`);
  ok(!(await mems(page)), `${tag} …and closed here`);
  // ── the window refuses: its own text is not saved ──
  await w.evaluate(() => window.__mock.ideaFailWrites("the disk is full"));
  await w.locator(".cm-content").click();
  await w.keyboard.type("DRAFT ");
  await until(w, () => !!document.querySelector("[data-testid=ideas-error]"));
  await page.bringToFront();
  await noWindow(page);
  await idea(page, GPU);
  await floatBtn(page).click();
  ok(await says(page, /The Ideas window has “.*” open with text that is not saved\. “Home server GPU” is still here\./) && await isDoc(page, GPU), `${tag} a refusal there is said here, and nothing moves`);
  ok(/DRAFT/.test(await w.locator(".cm-content").innerText()) && /Jellyfin|DRAFT/.test(await w.locator(".ix-title").innerText()), `${tag} …and its draft is still open there`);
  await w.close();
}

async function memories(page, tag, name) {
  const ctx = page.context();
  const ws = await main(page);
  await memory(page, "topics/nas-layout.md");
  const [w] = await Promise.all([ctx.waitForEvent("page"), floatBtn(page).click()]);
  ok(await until(w, () => document.querySelector("[data-testid=main-doc]")?.dataset.doc === "topics/nas-layout.md", null, 10000), `${tag} a memory file floats`);
  ok(await card(page) && /NAS layout/i.test(await page.locator("[data-testid=ws-floated]").innerText()), `${tag} …and its card is here`);
  ok(await until(w, () => document.querySelectorAll("[data-testid=ideas-memory-side] [data-testid=memory-file]").length >= 5) && !(await w.$("[data-testid=ideas-list]")), `${tag} with the folder's files beside it`);
  ok(await w.evaluate(() => !document.querySelector("[data-testid=ideas-relate], [data-testid=ideas-merge], [data-testid=ideas-chat-toggle], [data-testid=ideas-remove]")), `${tag} and none of an idea's acts`);
  if (SHOTS) await w.screenshot({ path: `${SHOTS}/df-memory-${name}.png` });
  // ── its neighbours, its History, a picture, an idea's link ──
  await w.locator("[data-testid=memory-file][data-path='MEMORY.md']").click();
  ok(await isDoc(w, "MEMORY.md"), `${tag} another file of it opens`);
  await w.click("[data-testid=ideas-versions-toggle], [data-testid=doc-menu-open]");
  if (!(await w.$("[data-testid=memory-history]"))) await w.click("[data-testid=doc-menu-history]");
  ok(await until(w, () => document.querySelectorAll("[data-testid=memory-log-row]").length > 0), `${tag} its History`);
  await w.click("[data-testid=memory-hist-close]");
  await w.locator("[data-testid=memory-file][data-path='files/rack.svg']").click();
  ok(await until(w, () => document.querySelector("[data-testid=memory-preview] img")?.naturalWidth > 0), `${tag} a picture of it`);
  await w.locator("[data-testid=memory-file][data-path='topics/nas-layout.md']").click();
  await isDoc(w, "topics/nas-layout.md");
  // ── typed there ──
  await w.click("[data-testid=ideas-mode-edit]");
  await w.locator(".cm-content").click();
  await w.keyboard.type("THERE ");
  ok(await until(w, (id) => window.__TAURI_INTERNALS__.invoke("workspace_memory_file", { id, path: "topics/nas-layout.md" }).then((f) => f.content.includes("THERE ")), ws), `${tag} typed there, saved by the memory's own row`);
  // ── All ideas: a text that cannot be saved stays ──
  await w.evaluate((id) => { const t = window.__TAURI_INTERNALS__; window.__real = t.invoke; t.invoke = (c, a) => (c === "workspace_memory_write" ? Promise.reject("the disk is full") : window.__real(c, a)); void id; }, ws);
  await w.keyboard.type("STUCK ");
  await w.click("[data-testid=ideas-all]");
  ok(await until(w, () => !!document.querySelector("[data-testid=ideas-error]")) && /STUCK/.test(await w.locator(".cm-content").innerText()), `${tag} All ideas does not drop a text that cannot be saved`);
  // A float meanwhile is refused, and says which.
  await page.bringToFront();
  await noWindow(page);
  await idea(page, GPU);
  await floatBtn(page).click();
  ok(await says(page, /The Ideas window has “.*” open with text that is not saved/) && await isDoc(page, GPU), `${tag} …and a Float meanwhile is refused here`);
  // ── Dock ──
  await w.bringToFront();
  await w.evaluate(() => { window.__TAURI_INTERNALS__.invoke = window.__real; });
  await w.locator("[data-testid=ideas-error] button").first().click().catch(() => {});
  await until(w, () => /Saved/.test(document.querySelector("[data-testid=ideas-save]")?.textContent ?? ""));
  ok(/Dock/.test(await w.locator("[data-testid=main-doc-float]").innerText()), `${tag} it has Dock`);
  await w.click("[data-testid=main-doc-float]");
  ok(await isDoc(page, "topics/nas-layout.md") && await until(page, () => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === "memory") && !(await page.$("[data-testid=ws-floated]")), `${tag} Dock opens it in its workspace again`);
  await w.waitForEvent("close", { timeout: 4000 }).catch(() => {});
  ok(w.isClosed(), `${tag} …and the window that was opened for it closes, so a relaunch does not bring it back empty`);
}

/** A window the person was in stays: Dock takes the file, not their window. */
async function theirs(page, tag) {
  const ctx = page.context();
  await main(page);
  const [w] = await Promise.all([ctx.waitForEvent("page"), page.click("[data-testid=ideas-rail]")]);
  await w.waitForSelector("[data-testid=ideas-rows] li");
  await page.bringToFront();
  await noWindow(page);
  await memory(page);
  await floatBtn(page).click();
  ok(await isDoc(w, "MEMORY.md") && await card(page), `${tag} floated to a window that was open`);
  await w.bringToFront();
  await w.click("[data-testid=main-doc-float]");
  ok(await isDoc(page, "MEMORY.md") && await until(w, () => !document.querySelector("[data-testid=ideas-memory-side]") && !!document.querySelector("[data-testid=ideas-list]")), `${tag} Dock: the Ideas window is the ideas again`);
  await w.waitForTimeout(400);
  ok(!w.isClosed(), `${tag} …and stays open`);
  // Opened for a file, and then used for an idea: theirs too.
  await w.close();
  await page.bringToFront();
  await page.evaluate(() => { localStorage.clear(); });
  const p = await ctx.newPage();
  await main(p);
  await memory(p);
  const [v] = await Promise.all([ctx.waitForEvent("page"), floatBtn(p).click()]);
  await isDoc(v, "MEMORY.md");
  await v.click("[data-testid=ideas-all]");
  await v.locator(`.ix-row[data-idea='${JELLY}'] button`).first().click();
  await v.waitForSelector(".cm-content");
  await p.bringToFront();
  await noWindow(p);
  await p.click("[data-testid=ws-floated-here]");
  await isDoc(p, "MEMORY.md");
  await floatBtn(p).click();
  await isDoc(v, "MEMORY.md");
  await v.bringToFront();
  await v.click("[data-testid=main-doc-float]");
  await isDoc(p, "MEMORY.md");
  await v.waitForTimeout(400);
  ok(!v.isClosed(), `${tag} one opened for a file and then used for an idea stays too`);
}

async function docks(page, tag) {
  // Dock that the main window cannot take: it stays there.
  const ctx = page.context();
  await main(page);
  await memory(page);
  const [w] = await Promise.all([ctx.waitForEvent("page"), floatBtn(page).click()]);
  await isDoc(w, "MEMORY.md");
  await card(page);
  await page.bringToFront();
  await idea(page, GPU);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.evaluate(() => window.__mock.ideaFailWrites("the disk is full"));
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.type("HELD ");
  await until(page, () => !!document.querySelector("[data-testid=ideas-error]"));
  await w.bringToFront();
  await w.click("[data-testid=main-doc-float]");
  ok(await says(w, /The main window has a document open with text that is not saved\. “.*” is still here\./) && await isDoc(w, "MEMORY.md"), `${tag} Dock refused: said there, and it stays there`);
  await page.bringToFront();
  ok(/HELD/.test(await page.locator("[data-testid=main-doc] .cm-content").innerText()), `${tag} …and the main window's text is where it was`);
  await w.close();
}

async function betas(page, tag) {
  await main(page, ["beta_workspace_chats"]);
  await memory(page);
  ok(await floatBtn(page).count() === 0, `${tag} with Ideas off there is no window to float to, and no button`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [waits, fails, silent, ideas, memories, theirs, docks, betas]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag, name); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (SHOTS) for (const [i, p] of page.context().pages().entries()) await p.screenshot({ path: `${SHOTS}/df-${name}-${part.name}-${i}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\ndocuments-float-browser: ${failed} failure(s)` : "\ndocuments-float-browser: all good");
process.exit(failed ? 1 : 0);
