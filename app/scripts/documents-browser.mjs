// Browser gate for a document open in the MAIN window (app/src/doc/
// MainDocument.tsx, the documents lane), in Chromium AND WebKit against the
// mock. The Ideas window's use of the same pane is ideas-browser.mjs's.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/documents-browser.mjs
// It runs against a BUILT page as well (VITE_MOCK=1 vite build --outDir <dir>,
// the companion's into <dir>/companion, then vite preview).
// IDEAS_SCREENSHOTS=<dir> saves the window per engine and theme.
//
// What it holds:
//   - the editor is fetched on the first EDIT: a plan opens in Read and fetches
//     none, an idea opens in Edit; the last choice is remembered per kind; a
//     fetch that fails says so, with Read and the tabs working; a change that
//     arrives while it is out is what opens, and nothing is written;
//   - the plan and an idea chip open it in the workspace view, on their tabs;
//     the tabs are the way back (no Back in the pane), and a tab change saves;
//   - the pane's own behaviours, with this window's rows: autosave, the heading
//     naming the file, versions and restore, rename, Read, an edit elsewhere
//     (clean, and dirty → Keep mine), `[[`, a followed idea: link;
//   - leaving by the nav: saved, and still open behind it; with a conflict,
//     kept, said, and there again on the workspace's name;
//   - Float MOVES it: saved, closed here, then opened in the Ideas window, with a
//     card in its place; when the save fails nothing moves;
//   - the header is one row from 340 px to 1052 px, by the pane's own width, and
//     everything it folds is in its menu;
//   - the editor's keys are its own (⌘[ indents less and does not navigate,
//     ⌃B does not toggle the nav), and the beta gate hides it;
//   - layout (every control reachable, the editor fills, no sideways scroll)
//     and the six themes.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
const JELLY = "01JC8Z4K3M9QX2T7VB00000901";
const GPU = "01JC8Z4K3M9QX2T7VB00000902";
const TIDY = "01JC8Z4K3M9QX2T7VB00000903";
const MOD = process.platform === "darwin" ? "Meta" : "Control";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const file = (page, id) => page.evaluate((id) => window.__mock.ideaFile(id), id);
const writes = (page, id) => page.evaluate((id) => window.__mock.ideaWrites().filter((w) => w.id === id), id);
const editorText = (page) => page.evaluate(() => document.querySelector("[data-testid=main-doc] .cm-content")?.innerText ?? null);
const shown = (page) => page.evaluate(() => { const d = document.querySelector("[data-testid=main-doc]"); return !!d && !d.hidden; });
const title = (page) => page.evaluate(() => document.querySelector("[data-testid=main-doc] .ix-title")?.textContent ?? null);
const caretAfter = async (page, text) => {
  await page.locator("[data-testid=main-doc] .cm-line", { hasText: text }).first().click();
  await page.keyboard.press("End");
};
const saved = (page) => until(page, () => document.querySelector("[data-testid=main-doc] [data-testid=ideas-save]")?.dataset.state === "clean");
const openChip = async (page, id) => {
  if (!(await page.$("[data-testid=ws-overview]"))) await overview(page);
  await page.click(`[data-testid=ws-ov-idea][data-idea='${id}'] .ws-ov-chip-open`);
  return until(page, (id) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === id && document.querySelector("[data-testid=main-doc] .cm-content"), id);
};
/** A tab of the workspace view. */
const tab = (page, t) => page.click(`[data-testid=ws-tab][data-tab=${t}]`);
const onTab = (page, t) => page.evaluate((t) => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === t, t);
const overview = async (page) => { await tab(page, "overview"); return until(page, () => !!document.querySelector("[data-testid=ws-overview]")); };
/** An act in the pane's menu. */
const act = async (page, testId) => { await page.click("[data-testid=main-doc] [data-testid=doc-menu-open]"); await page.click(`[data-testid=doc-menu] [data-testid=${testId}]`); };
/** The editor's module, from the dev server or as a built chunk. */
const EDITOR = /\/doc\/editor\.ts|\/assets\/editor-/;
const fetched = (page) => page.evaluate((re) => performance.getEntriesByType("resource").some((r) => new RegExp(re).test(r.name)), `codemirror|${EDITOR.source}`);

/** A workspace with a plan and two ideas, its overview open. */
async function setup(page, query = "") {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", beta_ideas: true });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1${query}`);
  await page.waitForSelector("[data-testid=ws-group]");
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-overview]");
  const plan = (await call(page, "plan_make", { ideas: [JELLY], title: "Jellyfin plan" })).id;
  const ws = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name.startsWith("Everything") && w.id !== "everything-else")?.id;
  await page.click("[data-testid=ws-ov-link-plan]");
  await page.click(`[data-testid=ws-ov-plan-pick-row][data-doc='${plan}']`);
  for (const id of [JELLY, GPU]) {
    await page.waitForSelector("[data-testid=ws-ov-link-idea]");
    if (!(await page.$(`[data-testid=ws-ov-idea-pick-row][data-doc='${id}']`))) await page.click("[data-testid=ws-ov-link-idea]");
    await page.click(`[data-testid=ws-ov-idea-pick-row][data-doc='${id}']`);
    await page.waitForSelector(`[data-testid=ws-ov-idea][data-idea='${id}']`);
  }
  return { plan, ws };
}
/** Select a place in the nav, whatever is open. */
const pickPlace = (page) => page.locator("[data-testid=ws-group] li.row[data-slug]").first().click();
const pickWs = (page) => page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();

async function flow(page, tag, name) {
  const { plan } = await setup(page);
  ok(!(await fetched(page)), `${tag} the overview is up and the editor has not been fetched`);
  ok(await openChip(page, JELLY), `${tag} an idea chip opens it on the Ideas tab, in Edit`);
  ok(await fetched(page) && await onTab(page, "ideas"), `${tag} …which fetches the editor`);
  ok(await page.locator("[data-testid=main-doc-back]").count() === 0, `${tag} the pane has no Back: the tabs are the way back`);
  ok(await page.locator("[data-testid=ws-overview]").count() === 0 && (await title(page)) === "Jellyfin on the home server", `${tag} in place of the overview, under its own title`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/main-doc-${name}.png` });

  // ── save, and the heading names the file ──
  const before = (await file(page, JELLY)).content;
  await caretAfter(page, "Jellyfin on the home server");
  await page.keyboard.type("!");
  ok(await until(page, (id) => window.__mock.ideaWrites().some((w) => w.id === id), JELLY, 3000), `${tag} typing saves on its own after the pause`);
  await saved(page);
  ok((await file(page, JELLY)).content === before.replace("# Jellyfin on the home server", "# Jellyfin on the home server!"), `${tag} the saved file differs by the one typed character`);
  ok((await title(page)) === "Jellyfin on the home server!", `${tag} the header follows the heading`);
  await page.keyboard.type("?");
  await overview(page);
  ok(await until(page, () => document.querySelector("[data-testid=main-doc]")?.hidden === true) && await onTab(page, "overview"), `${tag} the Overview tab returns to it, and the document stays open behind it`);
  ok((await file(page, JELLY)).content.includes("server!?"), `${tag} …having saved, without waiting for the pause`);
  ok(await until(page, (id) => document.querySelector(`[data-testid=ws-ov-idea][data-idea='${id}']`)?.textContent.includes("server!?"), JELLY), `${tag} and its chip says the new title`);

  // ── the plan ──
  await page.click("[data-testid=ws-ov-plan]");
  ok(await until(page, (id) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === id && !!document.querySelector("[data-testid=main-doc] [data-testid=ideas-read] .md-h1"), plan) && await onTab(page, "plan"), `${tag} the plan's name opens it on the Plan tab, in Read`);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  ok(await until(page, () => !!document.activeElement?.closest("[data-testid=main-doc] .cm-content")), `${tag} Edit puts the cursor in it`);
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.press(`${MOD}+ArrowDown`);
  await page.keyboard.type("\nA line of the plan's own.");
  await until(page, (id) => window.__mock.ideaFile(id).content.includes("plan's own"), plan, 3000);
  ok((await file(page, plan)).content.includes("A line of the plan's own."), `${tag} and it is edited and saved here`);
  await pickWs(page);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-overview]") && document.querySelector("[data-testid=main-doc]")?.hidden === true), `${tag} the workspace's name in the nav returns to its overview too`);
  await tab(page, "plan");
  ok(await until(page, (id) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === id && !document.querySelector("[data-testid=main-doc]").hidden && !!document.querySelector("[data-testid=main-doc] .cm-content"), plan), `${tag} and the Plan tab is back on the plan, as it was left`);

  // ── history, rename, Read ──
  await openChip(page, JELLY);
  await act(page, "ideas-rename");
  await page.fill("[data-testid=ideas-rename-input]", "Media at home");
  await page.keyboard.press("Enter");
  ok(await until(page, (id) => window.__mock.ideaFile(id)?.file === "media-at-home.md", JELLY, 3000) && await until(page, () => document.querySelector("[data-testid=main-doc] .ix-title")?.textContent === "Media at home"), `${tag} Rename moves the file, the heading and the header`);
  await page.waitForTimeout(500);
  ok(!(await page.$("[data-testid=ideas-conflict]")), `${tag} its own change is not taken for an edit elsewhere`);
  ok((await page.locator("[data-testid=ideas-versions-toggle]").textContent()) === "History" && !/Versions/.test(await page.locator("[data-testid=main-doc]").innerText()), `${tag} its history is called History`);
  await page.click("[data-testid=ideas-versions-toggle]");
  // (The mock's rename keeps no version; an act made elsewhere does, and the
  // open list follows the change.)
  await call(page, "idea_relate", { id: JELLY, other: TIDY });
  ok(await until(page, () => document.querySelectorAll("[data-testid=ideas-version]").length > 0), `${tag} History lists what an act kept, as it happens`);
  ok(await until(page, (id) => document.querySelector("[data-testid=main-doc] .cm-content")?.innerText.includes(id), TIDY), `${tag} and the act's own text is in the editor`);
  await page.locator("[data-testid=ideas-version-restore]").first().click();
  ok((await file(page, JELLY)).content.includes(TIDY), `${tag} Restore asks first`);
  await page.locator("[data-testid=ideas-version-restore]").first().click();
  ok(await until(page, (a) => !window.__mock.ideaFile(a[0]).content.includes(a[1]), [JELLY, TIDY], 3000) && await until(page, (id) => !document.querySelector("[data-testid=main-doc] .cm-content")?.innerText.includes(id), TIDY), `${tag} and puts the version back in the file and the editor`);
  await page.click("[data-testid=ideas-versions-toggle]");
  await page.click("[data-testid=ideas-mode-read]");
  ok(await until(page, () => !!document.querySelector("[data-testid=main-doc] [data-testid=ideas-read] .md-h1")) && !(await page.locator("[data-testid=main-doc] [data-testid=ideas-read]").textContent()).includes("id:"), `${tag} Read renders the body`);
  await page.click("[data-testid=ideas-mode-edit]");

  // ── edited elsewhere (the Ideas window is one such): clean, then dirty ──
  const theirs = (await file(page, JELLY)).content.replace("which *box*", "which *box*, from the other window");
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: JELLY, c: theirs });
  ok(await until(page, () => document.querySelector("[data-testid=main-doc] .cm-content")?.innerText.includes("from the other window")) && !(await page.$("[data-testid=ideas-conflict]")), `${tag} a change from the other window is taken when this one is clean`);
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: JELLY, c: theirs.replace(/^# .*$/m, "# Jellyfin, named elsewhere") });
  ok(await until(page, () => document.querySelector("[data-testid=main-doc] .ix-title")?.textContent === "Jellyfin, named elsewhere"), `${tag} and its heading, changed there, names it here`);
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: JELLY, c: theirs });
  await until(page, () => document.querySelector("[data-testid=main-doc] .cm-content")?.innerText.includes("Media at home"));
  await caretAfter(page, "from the other window");
  await page.keyboard.type(" MINE");
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: JELLY, c: theirs.replace("transcode", "THEIRS transcode") });
  ok(await until(page, () => !!document.querySelector("[data-testid=ideas-conflict]")) && (await editorText(page)).includes(" MINE") && (await file(page, JELLY)).content.includes("THEIRS"), `${tag} and is a conflict when it is not: both texts kept`);
  for (const t of ["overview", "plan"]) {
    await tab(page, t);
    await page.waitForTimeout(200);
    ok(await shown(page) && await onTab(page, "ideas") && (await page.locator("[data-testid=main-doc] [data-testid=ideas-notice]").textContent()).includes("not saved"), `${tag} the ${t} tab is refused, and says why`);
  }
  await page.click("[data-testid=main-doc-float]");
  await page.waitForTimeout(200);
  ok(await shown(page) && page.context().pages().length === 1 && !(await page.$("[data-testid=ws-floated]")), `${tag} and so is Float: nothing moves`);

  // ── away with a conflict: kept, said, and there again ──
  await pickPlace(page);
  ok(await until(page, () => document.querySelector("[data-testid=main-doc]")?.hidden === true), `${tag} a place picked in the nav covers it`);
  await page.waitForTimeout(400);
  ok(await page.locator("[data-testid=main-doc]").count() === 1, `${tag} its unsaved text stays open behind it`);
  ok((await page.locator("body").innerText()).includes("has text that is not saved"), `${tag} and the window says so`);
  await pickWs(page);
  ok(await until(page, () => { const d = document.querySelector("[data-testid=main-doc]"); return d && !d.hidden && !!d.querySelector("[data-testid=ideas-conflict]"); }) && (await editorText(page)).includes(" MINE"), `${tag} the workspace's name goes back to it, with its banner and its text`);
  ok(await until(page, () => !document.querySelector(".err-float")?.textContent.includes("not saved")), `${tag} and the window's own notice has gone`);
  await page.click("[data-testid=ideas-keep-mine]");
  ok(await until(page, (id) => window.__mock.ideaFile(id).content.includes(" MINE"), JELLY, 3000) && (await page.evaluate((id) => window.__mock.ideaHistory(id), JELLY)).some((h) => h.content.includes("THEIRS")), `${tag} Keep mine writes it, theirs in history`);
  await saved(page);

  // ── away clean, mid-typing ──
  await caretAfter(page, "MINE");
  await page.keyboard.type(" and more");
  await pickPlace(page);
  ok(await until(page, (id) => window.__mock.ideaFile(id).content.includes("MINE and more"), JELLY, 1000) && await until(page, () => document.querySelector("[data-testid=main-doc]")?.hidden === true), `${tag} a place picked mid-typing: saved at once, and still open behind it`);
  ok(!(await page.locator("body").innerText()).includes("has text that is not saved"), `${tag} …and nothing to say about it`);
  await pickWs(page);
  await page.waitForSelector("[data-testid=ws-overview]");

  // ── links ──
  await openChip(page, GPU);
  ok((await file(page, GPU)).content.includes("\r\n"), `${tag} the GPU fixture is CRLF`);
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.press(`${MOD}+ArrowDown`);
  await page.keyboard.type(" [[Media lib");
  ok(await until(page, () => document.querySelectorAll("[data-testid=ideas-link-pick]").length === 1), `${tag} [[ offers the other documents`);
  await page.keyboard.press("Enter");
  ok(await until(page, (a) => window.__mock.ideaFile(a[0]).content.includes(`(idea:${a[1]})`), [GPU, TIDY], 3000), `${tag} and Enter links one`);
  const gpu = (await file(page, GPU)).content;
  ok(!/[^\r]\n/.test(gpu), `${tag} a CRLF file stays CRLF`);
  await page.click("[data-testid=ideas-mode-read]");
  await page.locator("[data-testid=main-doc] [data-testid=ideas-read] a").last().click();
  ok(await until(page, (id) => document.querySelector(`[data-testid=ws-idea-row].on`)?.dataset.idea === id && "unlinked" in document.querySelector("[data-testid=ws-idea-row].on").dataset, TIDY), `${tag} a followed idea that is not linked is marked so, with Link`);
  ok(await until(page, (id) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === id, TIDY) && (await title(page)) === "Media library tidy", `${tag} a link in Read is followed, in this pane`);
  await page.click("[data-testid=ideas-mode-edit]");

  // ── the editor's keys are its own ──
  await caretAfter(page, "Media library tidy");
  const nav0 = await page.evaluate(() => document.querySelector("[data-testid=ws-group]")?.getBoundingClientRect().width ?? 0);
  await page.evaluate(() => { window.__heard = []; window.addEventListener("keydown", (e) => window.__heard.push(`${e.metaKey ? "M" : ""}${e.ctrlKey ? "C" : ""}${e.key}`)); });
  await page.keyboard.press("Meta+BracketLeft");
  await page.keyboard.press("Control+b");
  await page.waitForTimeout(400);
  const nav1 = await page.evaluate(() => document.querySelector("[data-testid=ws-group]")?.getBoundingClientRect().width ?? 0);
  const heard = await page.evaluate(() => window.__heard.filter((k) => /^M\[|^Cb/.test(k)));
  const mac = process.platform === "darwin";
  ok(await shown(page) && (!mac || (nav0 === nav1 && heard.length === 0)), `${tag} ⌘[ and ⌃B in the editor reach no window chord (${nav0} → ${nav1}, heard ${JSON.stringify(heard)})`);

  // ── Float ──
  await caretAfter(page, "Media library tidy");
  await page.keyboard.type(" floated");
  const [popup] = await Promise.all([page.context().waitForEvent("page"), page.click("[data-testid=main-doc-float]")]);
  ok((await file(page, TIDY)).content.includes("tidy floated"), `${tag} Float saves first`);
  ok(await until(popup, (id) => document.querySelector(".ix-row.on")?.dataset.idea === id && !!document.querySelector(".cm-content"), TIDY, 15000), `${tag} and the Ideas window opens the same document`);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-floated]") && !document.querySelector("[data-testid=main-doc]")) && /tidy floated is in the Ideas window/.test(await page.locator("[data-testid=ws-floated]").innerText()), `${tag} which is closed here: a card says where it went`);
  ok(await page.locator("[data-testid=ws-floated] button").evaluateAll((bs) => bs.map((b) => b.textContent).join()) === "Show the window,Open it here", `${tag} …with Show the window and Open it here`);
  await popup.close();
  await overview(page);
  ok(await onTab(page, "overview"), `${tag} the other tabs work meanwhile`);
  await tab(page, "ideas");
  await page.click(`[data-testid=ws-idea-row][data-idea='${GPU}'] .wsf-idea-open`);
  ok(await until(page, (id) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === id, GPU), `${tag} another idea opens here as ever`);
  await overview(page);
  await page.click("[data-testid=ws-ov-plan]");
  const [again] = await Promise.all([page.context().waitForEvent("page"), page.click("[data-testid=main-doc-float]")]);
  ok(await until(again, (id) => document.querySelector(".ix-row.on")?.dataset.idea === id || document.querySelector("[data-testid=ideas-notice]"), plan, 15000), `${tag} the plan floats too`);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-doc-plan] .wsf-chip-f")), `${tag} and its chip in the nav wears the mark`);
  await again.close();
  await page.click("[data-testid=ws-floated-here]");
  ok(await until(page, (id) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === id, plan) && await until(page, () => !document.querySelector(".wsf-chip-f")), `${tag} Open it here opens it, and the mark goes`);

  // ── Float with a save that fails ──
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.evaluate(() => window.__mock.ideaFailWrites("the disk is full"));
  await page.keyboard.type("x");
  await page.click("[data-testid=main-doc-float]");
  ok(await until(page, () => document.querySelector("[data-testid=ideas-error]")?.textContent.includes("the disk is full")), `${tag} Float with a save that fails says so`);
  ok(await shown(page) && page.context().pages().length === 1 && !(await page.$("[data-testid=ws-floated]")), `${tag} …and nothing moves`);
  await page.evaluate(() => window.__mock.ideaFailWrites(null));
  await page.locator("[data-testid=ideas-error] button").click();
  await saved(page);
  ok(await until(page, () => !document.querySelector("[data-testid=main-doc] [data-testid=ideas-notice]")), `${tag} saved, its notice goes too`);

  // ── the header, by the pane's own width ──
  const pane = page.locator("[data-testid=main-doc]");
  for (const w of [1052, 700, 640, 639, 484, 420, 419, 340]) {
    const m = await page.evaluate(async (w) => {
      const root = document.querySelector("[data-testid=main-doc]");
      Object.assign(root.style, { alignSelf: "flex-start", width: `${w}px` });
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const head = root.querySelector(".ix-head"), hb = head.getBoundingClientRect();
      const vis = (s) => { const e = head.querySelector(s); return !!e && e.getBoundingClientRect().width > 0; };
      const out = [...head.querySelectorAll("button, .ix-title")].filter((e) => { const q = e.getBoundingClientRect(); return q.width > 0 && (q.right > hb.right + 0.5 || q.left < hb.left - 0.5); }).length;
      const buttons = [...head.querySelectorAll("button")].map((e) => { const q = e.getBoundingClientRect(); return [q.width, q.height, q.top]; });
      const row = buttons.every(([bw, bh]) => bw > 0 && bh < 36);
      const r = { h: hb.height, out, row, title: head.querySelector(".ix-title").getBoundingClientRect().width, state: vis("[data-testid=ideas-save]"), hist: vis("[data-testid=ideas-versions-toggle]"), float: vis("[data-testid=main-doc-float]"), more: vis("[data-testid=doc-menu-open]"), seg: vis(".ix-seg") };
      Object.assign(root.style, { alignSelf: "", width: "" });
      return r;
    }, w);
    ok(m.h <= 45 && m.out === 0 && m.row && m.title > 30 && m.seg && m.float && m.more && m.hist === (w >= 640) && m.state === (w >= 420),
      `${tag} at ${w}px the header is one row of what belongs there (${JSON.stringify(m)})`);
  }
  // Folded, it is all in the menu; and text that is not saved always says so.
  await page.evaluate(() => Object.assign(document.querySelector("[data-testid=main-doc]").style, { alignSelf: "flex-start", width: "340px" }));
  await page.waitForTimeout(150);
  await act(page, "doc-menu-history");
  ok(await until(page, () => !!document.querySelector("[data-testid=main-doc] [data-testid=ideas-versions]")), `${tag} under 640 History is in the menu`);
  await act(page, "doc-menu-history");
  ok(!(await page.$("[data-testid=ideas-versions]")), `${tag} …and goes the same way`);
  await page.click("[data-testid=doc-menu-open]");
  await page.keyboard.press("Escape");
  ok(!(await page.$("[data-testid=doc-menu]")), `${tag} Escape closes the menu`);
  await page.evaluate(() => window.__mock.ideaFailWrites("the disk is full"));
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.type("y");
  ok(await until(page, () => { const s = document.querySelector("[data-testid=main-doc] [data-testid=ideas-save]"); return s.textContent === "Not saved" && s.getBoundingClientRect().width > 0; }), `${tag} at 340px "Not saved" still shows`);
  await page.evaluate(() => { window.__mock.ideaFailWrites(null); Object.assign(document.querySelector("[data-testid=main-doc]").style, { alignSelf: "", width: "" }); });
  await page.locator("[data-testid=ideas-error] button").click();
  await saved(page);
  ok(await pane.count() === 1, `${tag} one pane`);

  // ── layout ──
  for (const [w, h] of [[1500, 900], [1200, 800], [900, 600]]) {
    await page.setViewportSize({ width: w, height: h });
    await page.waitForTimeout(300);
    const lay = await page.evaluate(() => {
      const root = document.querySelector("[data-testid=main-doc]");
      const b = root.querySelector(".ix-body").getBoundingClientRect(), e = root.querySelector(".ix-editor").getBoundingClientRect(), m = document.querySelector(".main").getBoundingClientRect();
      const miss = [...root.querySelectorAll(".ix-head button")].filter((x) => { const q = x.getBoundingClientRect(); const hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2); return !(hit && (x === hit || x.contains(hit))); }).map((x) => x.textContent);
      const side = document.querySelector("[data-testid=ws-side]")?.getBoundingClientRect();
      return { fill: Math.abs(b.width - e.width) < 1 && Math.abs(b.height - e.height) < 1, wide: Math.abs(m.width - b.width - (side && side.right <= b.left + 1 ? side.width : 0)) < 1 && b.bottom <= m.bottom + 1, h: b.height, miss, scroll: document.documentElement.scrollWidth <= innerWidth };
    });
    ok(lay.fill && lay.wide && lay.h > 250 && lay.scroll && lay.miss.length === 0, `${tag} at ${w}×${h} the editor fills the main area and every control is reachable (${JSON.stringify(lay)})`);
  }
  await page.setViewportSize({ width: 1500, height: 900 });

  // ── the six themes ──
  for (const theme of THEMES) {
    await page.evaluate((t) => { if (t === "tokyo-night") delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t; }, theme);
    const low = await page.evaluate(() => {
      const parse = (c) => {
        let x = /^rgba?\(([^)]+)\)/.exec(c);
        if (x) { const p = x[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
        x = /^color\(srgb ([^)]+)\)/.exec(c);
        if (x) { const p = x[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; }
        return [0, 0, 0, 0];
      };
      const over = (top, under) => [0, 1, 2].map((i) => top[i] * top[3] + under[i] * (1 - top[3])).concat(1);
      const bgOf = (el) => { const st = []; for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) st.push(c); if (c[3] >= 1) break; } let acc = [255, 255, 255, 1]; for (const c of st.reverse()) acc = over(c, acc); return acc; };
      const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
      const ratio = (el) => { const bg = bgOf(el); const [a, b] = [lum(over(parse(getComputedStyle(el).color), bg)), lum(bg)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
      return [...document.querySelectorAll("[data-testid=main-doc] .ix-btn, [data-testid=main-doc] .ix-seg > button, [data-testid=main-doc] .ix-state, [data-testid=main-doc] .ix-title")].filter((e) => e.textContent.trim() && !e.disabled).map((e) => [e.textContent.trim().slice(0, 20), ratio(e)]).filter(([, r]) => r < 4.5);
    });
    ok(low.length === 0, `${tag} ${theme}: every word in the header reads ≥ 4.5:1${low.length ? `: ${low.map(([t, r]) => `${t} ${r.toFixed(2)}`).join("; ")}` : ""}`);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/main-doc-${name}-${theme}.png` });
  }
}

/** The editor is slow to come, then does not come at all. */
async function slow(page, tag) {
  let release;
  const hold = new Promise((r) => { release = r; });
  await page.route(EDITOR, async (r) => { await hold; return r.continue(); });
  await setup(page);
  await page.click(`[data-testid=ws-ov-idea][data-idea='${JELLY}'] .ws-ov-chip-open`);
  ok(await until(page, () => document.querySelector("[data-testid=main-doc] .ix-editor-wait")?.textContent.includes("Opening")), `${tag} while the editor is fetched, the pane says so`);
  const theirs = (await file(page, JELLY)).content.replace("which *box*", "which *box*, changed while it loaded");
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: JELLY, c: theirs });
  await page.waitForTimeout(700);
  release();
  ok(await until(page, () => document.querySelector("[data-testid=main-doc] .cm-content")?.innerText.includes("changed while it loaded")), `${tag} a change that came meanwhile is what opens`);
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.locator("[data-testid=main-doc] .ix-title").click();
  await page.waitForTimeout(1200);
  ok((await writes(page, JELLY)).length === 0 && (await file(page, JELLY)).content === theirs, `${tag} and nothing is written over it`);
}
async function broken(page, tag) {
  await page.route(EDITOR, (r) => r.abort());
  await setup(page);
  await page.click(`[data-testid=ws-ov-idea][data-idea='${JELLY}'] .ws-ov-chip-open`);
  ok(await until(page, () => !!document.querySelector("[data-testid=ideas-editor-error]")), `${tag} an editor that cannot be loaded says so`);
  await page.click("[data-testid=ideas-mode-read]");
  ok(await until(page, () => !!document.querySelector("[data-testid=main-doc] [data-testid=ideas-read] .md-h1")), `${tag} and Read still works`);
  ok(await overview(page) && (await writes(page, JELLY)).length === 0, `${tag} and so do the tabs, with nothing written`);
}
/** A document that is only read fetches no editor; the choice is kept per kind. */
async function readFirst(page, tag) {
  const { plan } = await setup(page);
  await page.click("[data-testid=ws-doc-plan]");
  ok(await until(page, () => !!document.querySelector("[data-testid=main-doc] [data-testid=ideas-read] .md-h1")), `${tag} a plan opens in Read`);
  await page.waitForTimeout(500);
  ok(!(await fetched(page)) && await page.locator("[data-testid=main-doc] [data-testid=ideas-editor]").count() === 0, `${tag} …which fetches no editor, and mounts none`);
  const theirs = (await file(page, plan)).content + "\nA line from elsewhere.\n";
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: plan, c: theirs });
  ok(await until(page, () => document.querySelector("[data-testid=main-doc] [data-testid=ideas-read]")?.textContent.includes("A line from elsewhere.")), `${tag} a change made elsewhere is read as it lands`);
  await page.locator("[data-testid=main-doc] [data-testid=ideas-read] a").first().click();
  ok(await until(page, (id) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === id && !!document.querySelector("[data-testid=main-doc] .cm-content"), JELLY) && await onTab(page, "ideas"), `${tag} its link to an idea opens the idea, on the Ideas tab, in Edit`);
  ok(await fetched(page), `${tag} …and that is when the editor is fetched`);
  await tab(page, "plan");
  ok(await until(page, () => !!document.querySelector("[data-testid=main-doc] [data-testid=ideas-read] .md-h1")), `${tag} the plan is in Read still`);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  ok(await until(page, () => document.querySelector("[data-testid=main-doc] .cm-content")?.innerText.includes("A line from elsewhere.")), `${tag} Edit has the text as it is now`);
  await tab(page, "ideas");
  await until(page, (id) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === id, JELLY);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-read]");
  await page.reload();
  await page.waitForSelector("[data-testid=ws-group]");
  ok(JSON.stringify(await page.evaluate(() => ["idea", "plan"].map((k) => localStorage.getItem(`wt.doc.mode.${k}`)))) === '["read","edit"]', `${tag} the last choice is kept, per kind`);
}
/** Ideas off: no overview, no document. Turned off with one open, it is gone. */
async function gate(page, tag) {
  await setup(page);
  await openChip(page, GPU);
  await caretAfter(page, "Home server GPU");
  await page.keyboard.type("!");
  await page.keyboard.press("Meta+Comma");
  await page.waitForSelector("[data-testid=beta-ideas]");
  await page.click("[data-testid=beta-ideas]");
  ok(await until(page, () => !document.querySelector("[data-testid=main-doc]") && !document.querySelector("[data-testid=ws-frame]")) && (await file(page, GPU)).content.includes("GPU!"), `${tag} turning Ideas off saves the open document and takes it away`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [flow, readFirst, slow, broken, gate]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1500, height: 900 } })).newPage();
    const errors = [];
    // xterm throws this on ANY place selected in the dev harness (plans-browser.mjs).
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag, name); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (SHOTS) await page.screenshot({ path: `${SHOTS}/main-doc-${name}-${part.name}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\ndocuments-browser: ${failed} failure(s)` : "\ndocuments-browser: all good");
process.exit(failed ? 1 : 0);
