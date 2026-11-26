// Browser gate for the workspace view (docs/proposals/workspace-view-design.md
// §2, §3: WorkspaceFrame.tsx, WorkspaceOverview.tsx), in Chromium AND WebKit
// against the mock. The document in it is documents-browser.mjs's.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/workspace-view-browser.mjs
// IDEAS_SCREENSHOTS=<dir> saves the views per engine.
//
// What it holds:
//   - which tabs, chips and cards each beta draws, and that with neither a
//     workspace's name folds its card;
//   - the frame: Overview first, the counts, Home not lit, the card's dot
//     waiting before working, the documents row (dashed, marked, one row at the
//     narrowest nav);
//   - the attention line, the empty states, every act of the old overview;
//   - Back and Forward through tabs, ideas, workspaces and places, and a Back
//     that a text which cannot be saved refuses;
//   - the document's section is ONE element with ONE parent through tabs,
//     places, Home and another workspace, and an unsaved text waits in it;
//   - layout and six themes.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
const JELLY = "01JC8Z4K3M9QX2T7VB00000901", GPU = "01JC8Z4K3M9QX2T7VB00000902";
const BODY = `# Plan: Jellyfin

From [Jellyfin](idea:${JELLY}).

## Goal

Media for the family.

## Projects

- worktrees (registered)
- ans (clone: https://github.com/demo/ans.git)

## Work

### NAS mount unit
project: worktrees
branch: feat/nas
brief: |
  Goal: the mount.

### Ansible role for Jellyfin
project: worktrees
branch: feat/role
brief: |
  Goal: the role.
`;
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const tab = (page, t) => page.click(`[data-testid=ws-tab][data-tab=${t}]`);
const isTab = (page, t) => until(page, (t) => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === t, t, 2000);
const isDoc = (page, id) => until(page, (id) => { const d = document.querySelector("[data-testid=main-doc]"); return !!d && !d.hidden && d.dataset.doc === id; }, id, 4000);
const tabs = (page) => page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-tab]")].map((t) => t.dataset.tab).join());
const group = (page, name) => page.locator("[data-testid=ws-group]", { has: page.locator("[data-testid=ws-name]", { hasText: name }) }).first();
const pickWs = (page, name = "Everything") => page.locator("[data-testid=ws-name]", { hasText: name }).first().click();
const pickPlace = (page, n = 0) => page.locator("[data-testid=ws-group] li.row[data-slug]").nth(n).click();
const homeLit = (page) => page.evaluate(() => document.querySelector(".home-item").classList.contains("on"));
const back = (page) => page.locator("[data-testid=nav-back]:visible").first().click();
const forward = (page) => page.locator("[data-testid=nav-forward]:visible").first().click();

async function boot(page, betas, extra = {}) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  // `restore_last: false`: these parts boot at Home (it is on by default since
  // 2026-11-25, and a fresh mock boot would otherwise open a place).
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", restore_last: false, ...Object.fromEntries(betas.map((b) => [b, true])), ...extra });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0`);
  await page.waitForSelector("[data-testid=ws-group]");
}
/** The workspace with a plan and two ideas linked, its Overview up. */
async function setup(page, betas = ["beta_ideas"], extra = {}) {
  await boot(page, betas, extra);
  const plan = (await call(page, "plan_make", { ideas: [JELLY], draft: BODY })).id;
  const ws = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name.startsWith("Everything"));
  await pickWs(page);
  await page.waitForSelector("[data-testid=ws-overview]");
  await page.click("[data-testid=ws-ov-link-plan]");
  await page.click(`[data-testid=ws-ov-plan-pick-row][data-doc='${plan}']`);
  for (const id of [JELLY, GPU]) {
    await page.click("[data-testid=ws-ov-link-idea]");
    await page.click(`[data-testid=ws-ov-idea-pick-row][data-doc='${id}']`);
    await page.waitForSelector(`[data-testid=ws-ov-idea][data-idea='${id}']`);
  }
  await page.waitForSelector("[data-testid=ws-ov-work]");
  return { plan, ws: ws.id };
}

async function gates(page, tag) {
  await boot(page, []);
  await pickWs(page);
  await page.waitForTimeout(200);
  ok(!(await page.$("[data-testid=ws-frame]")) && !(await page.$("[data-testid=ws-docs]")) && await group(page, "Everything").locator(".ws-kids").count() === 0, `${tag} with neither beta a workspace's name folds its card, and there is no view`);
  ok(await homeLit(page), `${tag} …and Home is lit`);
  await page.waitForTimeout(1200); // the app's own save of the fold
  await page.evaluate(() => { const s = JSON.parse(sessionStorage.getItem("wt-mock-ui-state")); sessionStorage.setItem("wt-mock-ui-state", JSON.stringify({ ...s, beta_workspace_chats: true, ws_collapsed: {} })); });
  await page.reload();
  await page.waitForSelector("[data-testid=ws-docs]");
  await pickWs(page);
  await page.waitForSelector("[data-testid=ws-overview]");
  ok(await tabs(page) === "overview,memory,chats", `${tag} workspace chats alone: Overview, Memory, Chats`);
  ok(await page.locator("[data-testid=ws-docs]").first().locator("button").evaluateAll((b) => b.map((x) => x.dataset.testid).join()) === "ws-doc-memory", `${tag} …one chip, Memory, and no Plan or Ideas`);
  ok(!(await page.$("[data-testid=ws-ov-card-work]")) && !(await page.$("[data-testid=ws-ov-card-ideas]")) && !!(await page.$("[data-testid=ws-ov-card-memory]")) && !!(await page.$("[data-testid=ws-ov-card-chats]")) && !!(await page.$("[data-testid=ws-ov-card-projects]")), `${tag} …and an Overview of Memory, Chats and Projects`);
  ok(await page.locator("[data-testid=ideas-rail]").count() === 0, `${tag} …without the lightbulb: Ideas is its own switch`);
  const other = await page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-name]")].map((e) => e.textContent).find((n) => !n.startsWith("Everything")));
  await pickWs(page, other);
  ok(await page.locator("button", { hasText: /^New chat$/ }).evaluateAll((b) => b.filter((x) => x.getBoundingClientRect().width).map((x) => x.dataset.testid).join()) === "ws-new-chat" && !(await page.$("[data-testid=ws-frame-new-chat]")), `${tag} an empty Overview offers New chat once (and the nav's +)`);
  for (const [t, words, buttons] of [["chats", "No chats yet.", ["New chat"]]]) {
    await tab(page, t);
    const e = await page.evaluate((t) => { const el = document.querySelector(`[data-testid=ws-no-${t}]`); return el && { words: el.querySelector("p").textContent, b: [...el.querySelectorAll("button")].map((x) => [x.textContent, x.disabled, x.title]) }; }, t);
    ok(e?.words === words && JSON.stringify(e.b.map((x) => x[0])) === JSON.stringify(buttons) && e.b.find((x) => x[0] === "New chat")[1] === false, `${tag} ${t}'s empty state, with New chat (${JSON.stringify(e)})`);
  }
  await page.keyboard.press("Meta+Comma");
  const box = page.locator("[data-testid=beta-workspace-chats]");
  await box.waitFor();
  ok(await box.isChecked() && /still being built/i.test(await page.locator("[data-beta=beta_workspace_chats]").innerText()), `${tag} its checkbox is in Settings, and says it is still being built`);
  await box.click();
  ok(await until(page, () => !document.querySelector("[data-testid=ws-frame]") && !document.querySelector("[data-testid=ws-docs]")), `${tag} turned off, the view is gone at once`);
}

async function frame(page, tag, name) {
  const { plan, ws } = await setup(page, ["beta_ideas", "beta_workspace_chats"]);
  ok(await tabs(page) === "overview,memory,plan,ideas,chats" && await isTab(page, "overview"), `${tag} a workspace opens on Overview, first of its five tabs`);
  ok(await page.locator("[data-testid=ws-frame-name]").innerText() === "Everything" && /^3 projects · 2 chats live$/.test(await page.locator(".wsf-counts").innerText()) && await page.locator("[data-testid=ws-frame-new-chat]").count() === 1, `${tag} its bar: the name, its projects and live chats, New chat`);
  ok(!(await homeLit(page)), `${tag} Home is not lit while a workspace is shown`);
  ok(await group(page, "Everything").locator(".ws-icon").first().evaluate((e) => e.classList.contains("waiting")), `${tag} the card's dot is waiting, though lanes work in it too`);
  const chips = await page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-docs]")[0].querySelectorAll("button")].map((b) => [b.dataset.testid, b.classList.contains("none"), b.querySelector(".wsf-chip-n")?.textContent ?? ""]));
  ok(JSON.stringify(chips) === JSON.stringify([["ws-doc-memory", false, ""], ["ws-doc-plan", false, ""], ["ws-doc-ideas", false, "2"]]), `${tag} its chips: Memory, Plan, Ideas 2 (${JSON.stringify(chips)})`);
  ok(await page.evaluate(() => { const g = document.querySelector("[data-testid=ws-group]"); const c = g.querySelector("[data-testid=ws-docs]"), p = g.querySelector(".project"); return !!(c.compareDocumentPosition(p) & Node.DOCUMENT_POSITION_FOLLOWING); }), `${tag} …above its projects`);
  await page.click("[data-testid=ws-doc-memory]");
  ok(await isTab(page, "memory"), `${tag} a chip opens its tab`);
  if (SHOTS) { await tab(page, "overview"); await page.screenshot({ path: `${SHOTS}/wsv-overview-${name}.png` }); }

  // ── the overview's acts ──
  await tab(page, "overview");
  ok(await page.locator("[data-testid=ws-attention]").evaluateAll((a) => a.map((x) => x.dataset.kind).join()) === "chat", `${tag} a chat waits, and nothing else needs anybody: one attention line`);
  ok(await page.locator("[data-testid=ws-ov-card-work] [data-testid=ws-ov-review]").count() === 2 && await page.locator("[data-testid=ws-ov-card-work] [data-testid=ws-ov-link-place]").count() === 2, `${tag} the plan's work has Review… and Link a place…`);
  await page.locator("[data-testid=ws-ov-review]").first().click();
  ok(await page.waitForSelector("[data-testid=start-review]").then(() => true, () => false), `${tag} Review… opens the review`);
  await page.click("[data-testid=start-edit]");
  ok(await isDoc(page, plan) && await isTab(page, "plan") && await until(page, () => !!document.querySelector("[data-testid=main-doc] .cm-content")), `${tag} its Edit the plan opens the plan in Edit, whatever plans open in`);
  await tab(page, "overview");
  ok(await page.locator("[data-testid=ws-ov-here]").count() === 3 && /\d+ places?/.test(await page.locator("[data-testid=ws-ov-here]").first().innerText()), `${tag} its projects, with their places`);
  const gone = page.locator("[data-testid=ws-ov-here]:not([data-registered])");
  ok(/not registered/.test(await gone.innerText()), `${tag} one that left the registry says so`);
  await gone.locator("[data-testid=ws-ov-take-out]").click();
  ok(await until(page, () => document.querySelectorAll("[data-testid=ws-ov-here]").length === 2), `${tag} …and Take it out takes it out`);
  await page.click("[data-testid=ws-ov-add-project]");
  ok(!!(await page.$("[data-testid=ws-add-project]")), `${tag} Add ▸ opens the workspace's menu`);
  await page.keyboard.press("Escape");
  await page.click("[data-testid=ws-frame-menu]");
  ok(!!(await page.$("[data-testid=ws-add-project]")), `${tag} and so does the bar's ⋯`);
  await page.keyboard.press("Escape");
  await page.click("[data-testid=ws-ov-plan-projects]");
  ok(await isTab(page, "plan") && await isDoc(page, plan), `${tag} a project that is not here yet leads to the Plan tab`);
  if (!(await page.$("[data-testid=ws-ov-bring]"))) await page.click("[data-testid=ws-side-toggle]");
  await page.click("[data-testid=ws-ov-bring][data-name=ans]");
  ok(await page.waitForSelector("[data-testid=clone-dialog]").then(() => true, () => false) && await page.inputValue("[data-testid=clone-url]") === "https://github.com/demo/ans.git", `${tag} …where Clone… opens the dialog, filled in`);
  await page.keyboard.press("Escape");
  await until(page, () => !document.querySelector("[data-testid=clone-dialog]"));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsv-plan-${name}.png` });

  // ── the Ideas tab ──
  await page.click("[data-testid=ws-doc-ideas]");
  ok(await isTab(page, "ideas") && await isDoc(page, JELLY), `${tag} the Ideas chip opens the first linked idea`);
  await page.click(`[data-testid=ws-idea-row][data-idea='${GPU}'] .wsf-idea-open`);
  ok(await isDoc(page, GPU) && await page.evaluate((id) => document.querySelector("[data-testid=ws-idea-row].on")?.dataset.idea === id, GPU), `${tag} another is picked in the list, which marks it`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsv-ideas-${name}.png` });
  await tab(page, "plan");
  await tab(page, "ideas");
  ok(await isDoc(page, GPU), `${tag} and it is the one the tab comes back to`);

  // ── problems ──
  const broken = (await page.evaluate((id) => window.__mock.ideaFile(id).content, plan)).replace("### NAS mount unit", "loose words\n\n### NAS mount unit");
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: plan, c: broken });
  await tab(page, "overview");
  ok(await until(page, () => /The plan has a problem/.test(document.querySelector("[data-testid=ws-attention][data-kind=plan]")?.textContent ?? "")), `${tag} a plan with a problem is the attention line`);
  await page.click("[data-testid=ws-attention][data-kind=plan] [data-testid=ws-attention-go]");
  ok(await isTab(page, "plan") && await isDoc(page, plan), `${tag} …which opens the plan`);

  // ── unlink, the empty states, Make a plan ──
  await tab(page, "overview");
  await page.click("[data-testid=ws-ov-plan-unlink]");
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-ov-card-work] [data-testid=ws-ov-link-plan]")), `${tag} Unlink leaves No plan, Link a plan, Make a plan`);
  ok(await until(page, () => document.querySelector("[data-testid=ws-doc-plan]").classList.contains("none")), `${tag} and its chip is dashed`);
  await tab(page, "plan");
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-no-plan]") && !document.querySelector("[data-testid=main-doc]")), `${tag} the Plan tab with none says so, and holds no document`);
  await page.click("[data-testid=ws-ov-make-plan]");
  ok(await until(page, () => /^Plan:/.test(document.querySelector("[data-testid=main-doc] .ix-title")?.textContent ?? "")), `${tag} Make a plan makes one from the linked ideas, and opens it there`);
  ok((await call(page, "workspaces_list", {})).workspaces.find((w) => w.id === ws).plan !== plan, `${tag} …as this workspace's`);
  await tab(page, "ideas");
  for (const id of [JELLY, GPU]) await page.locator(`[data-testid=ws-idea-row][data-idea='${id}'] [data-testid=ws-ov-idea-unlink]`).click();
  ok(await until(page, () => /None linked/.test(document.querySelector("[data-testid=ws-side]")?.textContent ?? "")), `${tag} the Ideas tab with none says so`);
}

async function history(page, tag) {
  const { plan } = await setup(page);
  const step = async () => page.waitForTimeout(700); // past the coalesce window
  await step(); await page.click("[data-testid=ws-doc-plan]"); await isDoc(page, plan);
  await step(); await tab(page, "ideas"); await isDoc(page, JELLY);
  await step(); await page.click(`[data-testid=ws-idea-row][data-idea='${GPU}'] .wsf-idea-open`); await isDoc(page, GPU);
  await step(); await pickPlace(page); await until(page, () => !!document.querySelector(".space > .topbar .slug"));
  await step(); await page.click(".home-item"); await step();
  ok(/Back to /.test(await page.locator("[data-testid=nav-back]:visible").first().getAttribute("title")), `${tag} Home has somewhere to go back to`);
  await back(page);
  ok(await until(page, () => !!document.querySelector(".space > .topbar .slug")), `${tag} Back from Home is the place`);
  await step(); await back(page);
  ok(await isTab(page, "ideas") && await isDoc(page, GPU), `${tag} Back from the place is the idea it was left on`);
  ok(/Back to Everything · Ideas/.test(await page.locator("[data-testid=nav-back]:visible").first().getAttribute("title")), `${tag} the button names the workspace and the tab`);
  await step(); await back(page);
  ok(await isTab(page, "ideas") && await isDoc(page, JELLY), `${tag} Back again is the idea before it`);
  await step(); await back(page);
  ok(await isTab(page, "plan") && await isDoc(page, plan), `${tag} then the plan`);
  await step(); await back(page);
  ok(await isTab(page, "overview"), `${tag} then the Overview`);
  await step(); await forward(page); await step(); await forward(page);
  ok(await isTab(page, "ideas") && await isDoc(page, JELLY), `${tag} and Forward twice is the idea`);

  // ── a text that cannot be saved: Back is refused, and nothing moves ──
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.evaluate(() => window.__mock.ideaFailWrites("the disk is full"));
  await page.keyboard.type("z");
  await until(page, () => !!document.querySelector("[data-testid=ideas-error]"));
  const title = await page.locator("[data-testid=nav-back]:visible").first().getAttribute("title");
  await back(page);
  await page.waitForTimeout(1800); // past the apply's own deadline
  ok(await isTab(page, "ideas") && await isDoc(page, JELLY) && await page.locator("[data-testid=nav-back]:visible").first().getAttribute("title") === title && !(await page.locator("[data-testid=nav-forward]:visible").first().isDisabled()), `${tag} Back off a text that cannot be saved is refused, and the history is as it was`);
  // …Back to a PLACE is a nav act: it goes, and the text waits.
  await page.locator("[data-testid=nav-forward]:visible").first().click();
  await page.waitForTimeout(300);
  ok(await isTab(page, "ideas"), `${tag} Forward to another idea of it is refused the same way`);
  await page.evaluate(() => window.__mock.ideaFailWrites(null));
  await page.locator("[data-testid=ideas-error] button").click();
  await until(page, () => document.querySelector("[data-testid=main-doc] [data-testid=ideas-save]")?.dataset.state === "clean");
  await back(page);
  ok(await isTab(page, "plan"), `${tag} saved, it goes`);

  // ── Home is gone to, and a removed workspace is skipped ──
  await step(); await page.click(".home-item"); await step();
  ok(await homeLit(page) && !(await page.locator("[data-testid=ws-frame]").isVisible()), `${tag} Home leaves the workspace`);
  await group(page, "Everything").locator("[data-testid=ws-menu]").click();
  await page.click("[data-testid=ws-remove]");
  await page.click("[data-testid=ws-remove]");
  await until(page, () => ![...document.querySelectorAll("[data-testid=ws-name]")].some((e) => e.textContent === "Everything"));
  await step();
  ok(!/Everything/.test(await page.locator("[data-testid=nav-back]:visible").first().getAttribute("title")), `${tag} a removed workspace is nowhere to go back to`);
}

async function life(page, tag) {
  const { plan } = await setup(page, ["beta_ideas", "beta_workspace_chats"]);
  await page.click("[data-testid=ws-doc-plan]");
  await isDoc(page, plan);
  await page.evaluate(() => { window.__sec = document.querySelector("[data-testid=main-doc]"); window.__slot = window.__sec.parentElement; });
  const same = () => page.evaluate(() => { const s = document.querySelector("[data-testid=main-doc]"); return s === window.__sec && s.parentElement === window.__slot && s.isConnected; });
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.type("Typed in the plan.");
  await pickPlace(page);
  ok(await until(page, (id) => window.__mock.ideaFile(id).content.includes("Typed in the plan."), plan, 1000), `${tag} typed in a plan, then a place picked: it is saved at once`);
  ok(await same() && await page.evaluate(() => window.__sec.closest("[data-testid=ws-frame]").hidden), `${tag} the same section, in the same parent, hidden`);
  ok(await page.evaluate(() => { const f = document.querySelector("[data-testid=ws-frame]").getBoundingClientRect(); return f.width === 0 && f.height === 0; }), `${tag} …and the place has the whole room`);
  await page.click("[data-testid=ws-doc-plan]");
  ok(await isDoc(page, plan) && await same() && (await page.locator("[data-testid=main-doc] .cm-content").innerText()).includes("Typed in the plan."), `${tag} back on the Plan tab it is there, as it was left`);
  for (const t of ["overview", "memory", "ideas", "chats", "plan"]) { await tab(page, t); await isTab(page, t); if (!(await same())) ok(false, `${tag} the section moved on the ${t} tab`); }
  ok(await same(), `${tag} through every tab it is one element with one parent`);
  await page.click(".home-item");
  ok(await same(), `${tag} …through Home`);
  const other = await page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-name]")].map((e) => e.textContent).find((n) => !n.startsWith("Everything")));
  await pickWs(page, other);
  ok(await until(page, (n) => document.querySelector("[data-testid=ws-frame-name]")?.textContent === n, other) && await same(), `${tag} …and through another workspace (${other})`);

  // ── unsavable first ──
  await page.click("[data-testid=ws-doc-plan]");
  await isDoc(page, plan);
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.evaluate((id) => { const c = window.__mock.ideaFile(id).content; window.__theirs = c + "\nTheirs.\n"; }, plan);
  await page.keyboard.type(" MINE");
  await page.evaluate((id) => window.__mock.ideaEditElsewhere(id, window.__theirs), plan);
  ok(await until(page, () => !!document.querySelector("[data-testid=ideas-conflict]")), `${tag} a conflict: it cannot be saved`);
  await tab(page, "overview");
  await page.waitForTimeout(200);
  ok(await isTab(page, "plan"), `${tag} its tab does not change`);
  await pickPlace(page);
  ok(await until(page, () => /is still open in its workspace/i.test(document.body.innerText)) && await same(), `${tag} a place is not refused: the text waits behind it, and the window says so`);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-doc-plan] .wsf-chip-dot:not(.new)")), `${tag} its chip is marked`);
  await pickWs(page, other);
  ok(await until(page, () => /has text that is not saved/.test(document.querySelector("[data-testid=ws-attention][data-kind=unsaved]")?.textContent ?? "")) && /in Everything/.test(await page.locator("[data-testid=ws-attention][data-kind=unsaved]").innerText()), `${tag} another workspace's Overview says which document, and where`);
  await page.click("[data-testid=ws-attention-go]");
  ok(await isDoc(page, plan) && !!(await page.$("[data-testid=ideas-conflict]")) && (await page.locator("[data-testid=main-doc] .cm-content").innerText()).includes(" MINE") && await same(), `${tag} …and goes to it: the banner, and the text`);
  await pickPlace(page);
  await pickWs(page);
  ok(await isDoc(page, plan) && await isTab(page, "plan") && !!(await page.$("[data-testid=ideas-conflict]")), `${tag} its workspace's name goes to it too`);
  await page.click("[data-testid=ideas-keep-mine]");
  ok(await until(page, (id) => window.__mock.ideaFile(id).content.includes(" MINE"), plan) && await until(page, () => !document.querySelector(".wsf-chip-dot:not(.new)")), `${tag} Keep mine saves it, and the mark goes`);

  // ── a slow save, and a newer click: the newer one wins ──
  await page.evaluate(() => window.__mock.ideaSlowWrites(1200));
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.type(" SLOW");
  await tab(page, "overview");
  await pickPlace(page);
  await page.waitForTimeout(2000);
  ok(await page.locator(".space > .topbar .slug").isVisible() && !(await page.locator("[data-testid=ws-frame]").isVisible()) && (await page.evaluate((id) => window.__mock.ideaFile(id).content, plan)).includes(" SLOW"), `${tag} a tab asked for, then a place while its save is out: the place stays, and it is saved`);
  await page.click("[data-testid=ws-doc-plan]");
  await isDoc(page, plan);
  await page.evaluate(() => window.__mock.ideaSlowWrites(0));

  // ── removing the workspace of a text that cannot be saved ──
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.evaluate(() => window.__mock.ideaFailWrites("the disk is full"));
  await page.keyboard.type(" HELD");
  await until(page, () => !!document.querySelector("[data-testid=ideas-error]"));
  await pickPlace(page);
  const removeWs = async () => { await group(page, "Everything").locator("[data-testid=ws-menu]").click(); await page.click("[data-testid=ws-remove]"); await page.click("[data-testid=ws-remove]"); };
  await removeWs();
  ok(await isDoc(page, plan) && (await call(page, "workspaces_list", {})).workspaces.some((w) => w.name === "Everything"), `${tag} removing the workspace of a text that cannot be saved goes to the text instead`);
}

async function removed(page, tag) {
  const { plan } = await setup(page);
  await page.click("[data-testid=ws-doc-plan]");
  await isDoc(page, plan);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  // ── a workspace removed with a document of it open ──
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.type(" LAST");
  await group(page, "Everything").locator("[data-testid=ws-menu]").click();
  await page.click("[data-testid=ws-remove]");
  await page.click("[data-testid=ws-remove]");
  ok(await until(page, (id) => window.__mock.ideaFile(id).content.includes(" LAST") && !document.querySelector("[data-testid=main-doc]") && !document.querySelector("[data-testid=ws-frame]"), plan) && await homeLit(page), `${tag} its workspace removed: the document is saved, then closed, and Home is shown`);
}

/** …and removed elsewhere (another window) while its text cannot be saved. */
async function orphan(page, tag) {
  const { plan, ws } = await setup(page);
  await page.click("[data-testid=ws-doc-plan]");
  await isDoc(page, plan);
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.evaluate(() => window.__mock.ideaFailWrites("the disk is full"));
  await page.keyboard.type(" HELD");
  await until(page, () => !!document.querySelector("[data-testid=ideas-error]"));
  const other = await page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-name]")].map((e) => e.textContent).find((n) => !n.startsWith("Everything")));
  await pickWs(page, other);
  await call(page, "workspace_remove", { id: ws });
  // The window hears of it with the next answer that carries the list.
  await group(page, other).locator("[data-testid=ws-menu]").click();
  await page.click("[data-testid=ws-color]");
  await page.locator("[data-testid=ws-color-item][data-color=blue]").click();
  ok(await until(page, () => ![...document.querySelectorAll("[data-testid=ws-name]")].some((e) => e.textContent === "Everything")), `${tag} its workspace is gone`);
  await page.keyboard.press("Escape");
  await page.locator("[data-testid=ws-attention][data-kind=unsaved] [data-testid=ws-attention-go]").click();
  ok(await isDoc(page, plan) && !!(await page.$("[data-testid=ideas-error]")) && (await page.locator("[data-testid=main-doc] .cm-content").innerText()).includes(" HELD"), `${tag} the text is still reachable, with its banner, in the workspace shown`);
  await page.evaluate(() => window.__mock.ideaFailWrites(null));
  await page.locator("[data-testid=ideas-error] button").click();
  ok(await until(page, (id) => window.__mock.ideaFile(id).content.includes(" HELD"), plan), `${tag} and is saved from there`);
}

/** A workspace changed in the Ideas window (or by anybody else). */
async function elsewhere(page, tag) {
  await boot(page, ["beta_ideas"]);
  await page.click("[data-testid=ws-doc-plan]");
  ok(await until(page, () => /A plan is made from ideas: link one first\./.test(document.querySelector("[data-testid=ws-no-plan]")?.textContent ?? "")) && await page.locator("[data-testid=ws-ov-make-plan]").isDisabled(), `${tag} Make a plan with no idea linked says why, in words`);
  const ws = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
  await pickWs(page);
  await page.waitForSelector("[data-testid=ws-overview]");
  await call(page, "workspace_link_idea", { id: ws, idea: JELLY });
  ok(await page.evaluate(() => /None linked/.test(document.querySelector("[data-testid=ws-ov-card-ideas]").textContent)), `${tag} linked elsewhere: nothing here has heard`);
  await page.evaluate(() => window.dispatchEvent(new StorageEvent("storage", { key: "wt.workspaces.changed", newValue: "1" })));
  ok(await until(page, (id) => !!document.querySelector(`[data-testid=ws-ov-idea][data-idea='${id}']`) && !document.querySelector("[data-testid=ws-doc-ideas]").classList.contains("none"), JELLY), `${tag} the Ideas window's word brings it, on the Overview and the chip`);
  await call(page, "workspace_unlink_idea", { id: ws, idea: JELLY });
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  ok(await until(page, () => /None linked/.test(document.querySelector("[data-testid=ws-ov-card-ideas]")?.textContent ?? "")), `${tag} and so does coming to the front`);
  // The Ideas window says so: the real one, in a second page of this context.
  const other = await page.context().newPage();
  await other.addInitScript(() => { window.__said = 0; const set = Storage.prototype.setItem; Storage.prototype.setItem = function (k, v) { if (k === "wt.workspaces.changed") window.__said++; return set.call(this, k, v); }; });
  await other.goto(`${BASE}/companion/index.html?ideas=seed&workspaces=1&folding=0`);
  await other.click(`[data-idea='${JELLY}'] button.ix-row-main`, { button: "right" });
  await other.click("[data-testid=ideas-menu-link-ws]");
  await other.locator(".ix-dialog button, [data-testid=ideas-ws-pick-row]").filter({ hasText: "Everything" }).first().click();
  ok(await until(other, () => window.__said === 1), `${tag} Link to workspace… in the Ideas window says it`);
  await other.close();
}

/** Quit on a workspace's tab, and opened again. */
async function relaunch(page, tag) {
  for (const [on, want] of [[true, "ideas"], [false, null]]) {
    const p = await page.context().newPage();
    await p.addInitScript(([s]) => { if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s)); },
      [{ places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", beta_ideas: true, beta_workspace_chats: true, restore_last: on }]);
    await p.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0`);
    await p.waitForSelector("[data-testid=ws-group]");
    await p.waitForTimeout(800);
    await p.locator("[data-testid=ws-chat-row]", { hasText: "Release order" }).first().click();
    await p.click("[data-testid=ws-tab][data-tab=ideas]");
    await p.waitForTimeout(300);
    await p.reload();
    await p.waitForSelector("[data-testid=ws-group]");
    await p.waitForTimeout(800);
    const got = await p.evaluate(() => [document.querySelector("[data-testid=ws-tab].on")?.dataset.tab ?? null, document.querySelector(".wsc-name-t")?.textContent ?? null, document.querySelector("[data-testid=nav-back]")?.disabled]);
    ok(JSON.stringify(got.slice(0, 2)) === JSON.stringify(want ? ["ideas", "Release order"] : [null, null]), `${tag} restore last ${on ? "on: the workspace, its tab and its chat" : "off: Home, as for a place"} (${JSON.stringify(got)})`);
    if (on) ok(got[2] === true, `${tag} …and launching is not a step to go back from`);
    // Home is what is restored after Home.
    await p.click(".home-item");
    await p.waitForTimeout(200);
    ok(await p.evaluate(() => localStorage.getItem("wt.last.view")) === null, `${tag} leaving it forgets it`);
    await p.close();
  }
}

async function layout(page, tag, name) {
  await setup(page, ["beta_ideas", "beta_workspace_chats"]);
  for (const [w, h] of [[1600, 1000], [1440, 900], [1200, 800], [900, 700]]) {
    await page.setViewportSize({ width: w, height: h });
    for (const t of ["overview", "memory", "plan", "ideas", "chats"]) {
      await tab(page, t);
      await page.waitForTimeout(250);
      const miss = await page.evaluate(() => {
        const f = document.querySelector("[data-testid=ws-frame]"), fr = f.getBoundingClientRect();
        const bad = [];
        for (const b of f.querySelectorAll("button:not(:disabled)")) {
          const q0 = b.getBoundingClientRect();
          if (!q0.width || !q0.height) continue;
          b.scrollIntoView({ block: "nearest" });
          const q = b.getBoundingClientRect(), hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2);
          if (!(hit && (b === hit || b.contains(hit)))) bad.push(b.dataset.testid || b.textContent.trim());
          if (q.right > fr.right + 1) bad.push(`past the edge: ${b.textContent.trim()}`);
        }
        const main = document.querySelector(".main").getBoundingClientRect();
        return { bad, scroll: document.documentElement.scrollWidth <= innerWidth, fills: Math.abs(main.width - fr.width) < 1 && Math.abs(main.bottom - fr.bottom) < 1, wide: [...f.querySelectorAll("*")].filter((e) => e.scrollWidth > e.clientWidth + 1 && /auto|scroll/.test(getComputedStyle(e).overflowX) && !e.closest(".ix-body") && !e.classList.contains("wsf-tabs")).map((e) => e.className) };
      });
      ok(miss.bad.length === 0 && miss.scroll && miss.fills && miss.wide.length === 0, `${tag} ${w}px ${t}: every control is reachable and nothing scrolls sideways${miss.bad.length || !miss.fills || miss.wide.length ? ` (${JSON.stringify(miss)})` : ""}`);
    }
    const cols = await page.evaluate(() => { document.querySelector("[data-testid=ws-tab][data-tab=overview]").click(); return new Promise((r) => setTimeout(() => { const m = document.querySelector(".wsf-col-main").getBoundingClientRect(), s = document.querySelector(".wsf-col-side").getBoundingClientRect(); r({ beside: s.left >= m.right - 1, under: s.bottom <= m.top + 1, room: document.querySelector(".wsf-docs").getBoundingClientRect().width }); }, 250)); });
    ok(cols.room >= 900 ? cols.beside : cols.under, `${tag} ${w}px (${cols.room} of room): ${cols.room >= 900 ? "two columns" : "one column, memory last"}`);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsv-overview-${w}-${name}.png` });
  }
  // ── the narrowest nav ──
  await page.setViewportSize({ width: 1440, height: 900 });
  for (const [nav, icons] of [[300, true], [220, false]]) {
    await page.evaluate((n) => { const s = JSON.parse(sessionStorage.getItem("wt-mock-ui-state")); sessionStorage.setItem("wt-mock-ui-state", JSON.stringify({ ...s, nav_width: n })); }, nav);
    await page.reload();
    await page.waitForSelector("[data-testid=ws-docs]");
    const r = await page.evaluate(() => { const c = document.querySelector("[data-testid=ws-docs]"); const bs = [...c.querySelectorAll("button")]; return { n: bs.length, tops: new Set(bs.map((b) => Math.round(b.getBoundingClientRect().top))).size, icons: bs.some((b) => b.querySelector(".wsf-chip-i").getBoundingClientRect().width > 0), out: bs.some((b) => b.getBoundingClientRect().right > c.getBoundingClientRect().right), h: c.getBoundingClientRect().height, w: document.querySelector(".nav").getBoundingClientRect().width }; });
    ok(r.n === 3 && r.tops === 1 && r.icons === icons && !r.out, `${tag} a ${nav}px nav: the chips are one row, ${icons ? "with" : "without"} icons (${JSON.stringify(r)})`);
  }
}

async function themes(page, tag, name) {
  await setup(page, ["beta_ideas", "beta_workspace_chats"]);
  for (const theme of THEMES) {
    await page.evaluate((t) => { if (t === "tokyo-night") delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t; }, theme);
    for (const t of ["overview", "plan", "ideas"]) {
      await tab(page, t);
      await page.waitForTimeout(150);
      const low = await page.evaluate(() => {
        const parse = (c) => {
          let x = /^rgba?\(([^)]+)\)/.exec(c);
          if (x) { const p = x[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
          x = /^color\(srgb ([^)]+)\)/.exec(c);
          if (x) { const p = x[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; }
          return [0, 0, 0, 0];
        };
        const over = (a, b) => [0, 1, 2].map((i) => a[i] * a[3] + b[i] * (1 - a[3])).concat(1);
        const bgOf = (el) => { const st = []; for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) st.push(c); if (c[3] >= 1) break; } let acc = [255, 255, 255, 1]; for (const c of st.reverse()) acc = over(c, acc); return acc; };
        const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
        const ratio = (el) => { const bg = bgOf(el); const [a, b] = [lum(over(parse(getComputedStyle(el).color), bg)), lum(bg)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
        return [...document.querySelectorAll(".wsf-tab, .wsf-chip, .wsf-name-t, .wsf-counts, .wsf-card-name, .wsf-attn-t, .wsf-more, .wsf-link, .ws-ov-none, .ws-ov-name, .ws-ov-state, .wsf-idea-open, .wsf-side-line, [data-testid=ws-frame] .ctrl:not(:disabled), .wsf-empty p")]
          .filter((e) => e.textContent.trim() && e.getBoundingClientRect().width).map((e) => [e.className + ": " + e.textContent.trim().slice(0, 20), ratio(e)]).filter(([, r]) => r < 4.5);
      });
      ok(low.length === 0, `${tag} ${theme} ${t}: its words read ≥ 4.5:1${low.length ? `: ${low.map(([a, r]) => `${a} ${r.toFixed(2)}`).join("; ")}` : ""}`);
    }
    if (SHOTS) { await tab(page, "overview"); await page.screenshot({ path: `${SHOTS}/wsv-${theme}-${name}.png` }); }
  }
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [gates, frame, history, life, removed, orphan, elsewhere, relaunch, layout, themes]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const errors = [];
    // xterm throws this on ANY place selected in the dev harness (plans-browser.mjs).
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    page.on("console", (m) => { if (m.type() === "error" && /Maximum update depth|Cannot update a component|unique "key"/.test(m.text())) errors.push(m.text().slice(0, 200)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag, name); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsv-${name}-${part.name}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nworkspace-view-browser: ${failed} failure(s)` : "\nworkspace-view-browser: all good");
process.exit(failed ? 1 : 0);
