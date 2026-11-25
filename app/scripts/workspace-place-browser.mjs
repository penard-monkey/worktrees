// Browser gate for a place that knows its workspace (docs/proposals/
// workspace-view-design.md §6: the top bar's crumb, WorkspaceDock.tsx), in
// Chromium AND WebKit against the mock.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/workspace-place-browser.mjs
//
// What it holds: the card a place was clicked in is its workspace, through All
// projects, a relaunch and a project that leaves; a picker when several hold it and
// nobody said; the crumb goes back to where you were; the dock tab reads Memory, Plan
// and Ideas (each under its own beta), with no editor and no chat, links in place;
// Edit goes to the workspace view in Edit; both dock sides; layout and six themes.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
const JELLY = "01JC8Z4K3M9QX2T7VB00000901", GPU = "01JC8Z4K3M9QX2T7VB00000902";
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const card = (page, n) => page.locator("[data-testid=ws-group]", { has: page.locator("[data-testid=ws-name]", { hasText: n }) }).first();
const crumb = (page) => page.evaluate(() => document.querySelector("[data-testid=place-ws]")?.textContent.trim() ?? null);
const read = (page) => page.evaluate(() => document.querySelector("[data-testid=dock-ws-read]")?.innerText ?? null);
const rail = (page) => page.locator("[data-track='dock.workspace']").first();
const back = (page) => page.locator("[data-testid=nav-back]:visible").first().click({ timeout: 6000 });
const refocus = (page) => page.evaluate(() => window.dispatchEvent(new Event("focus")));
const isCrumb = (page, n) => until(page, (n) => document.querySelector("[data-testid=place-ws]")?.textContent.trim() === n, n);

async function boot(page, betas = ["beta_workspace_chats", "beta_ideas"], extra = {}) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", ...Object.fromEntries(betas.map((b) => [b, true])), ...extra });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0&memtick=150`);
  await page.waitForSelector("[data-testid=ws-group]");
  const l = (await call(page, "workspaces_list", {})).workspaces;
  const casa = l.find((w) => w.name === "Everything").projects.map((p) => p.root)[0];
  return { all: l.find((w) => w.name === "Everything"), other: l.find((w) => w.name !== "Everything" && w.id !== "everything-else"), root: casa };
}
const rowIn = (page, n, root) => card(page, n).locator(`[data-project-root='${root}'] li.row[data-slug]`).nth(1);

async function which(page, tag) {
  const { all, other, root } = await boot(page);
  const only = (await call(page, "workspaces_list", {})).workspaces.filter((w) => w.projects.some((p) => p.root === root)).length;
  if (only > 1) await call(page, "workspace_set_projects", { id: other.id, roots: [] });
  await refocus(page); await page.waitForTimeout(300);
  await rowIn(page, "Everything", root).click();
  ok(await isCrumb(page, "Everything"), `${tag} a place clicked in a card has its workspace in the bar`);
  ok(await page.evaluate(() => { const c = document.querySelector("[data-testid=place-ws]").getBoundingClientRect(), p = document.querySelector(".topbar-proj")?.getBoundingClientRect(); return !p || c.right <= p.left; }), `${tag} …before its project`);
  // A render in the middle of a press must not replace the row under it, or the
  // click goes to nothing: the row is the same node after one.
  const held = await rowIn(page, "Everything", root).elementHandle();
  await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# Memory\n\nAnother line.\n"), all.id);
  await page.waitForTimeout(600);
  ok(await held.evaluate((e) => e.isConnected), `${tag} a row is the same element after a render`);
  // All projects: the only one that holds it.
  await page.click("[data-testid=nav-view-projects]");
  const inAll = page.locator(`[data-project-root='${root}'] li.row[data-slug]`).nth(2);
  await inAll.click();
  ok(await isCrumb(page, "Everything"), `${tag} from All projects, the only workspace that holds it`);
  // In two: nobody said.
  await call(page, "workspace_set_projects", { id: other.id, roots: [root] });
  await refocus(page);
  ok(await isCrumb(page, "2 workspaces ▾"), `${tag} in two, it asks`);
  await rail(page).click();
  ok(await until(page, () => !!document.querySelector("[data-testid=dock-ws-pick]")), `${tag} and so does its dock tab`);
  await page.click("[data-testid=place-ws]");
  ok(await page.locator("[data-testid=place-ws-pick]").count() === 2, `${tag} the crumb's picker has both`);
  await page.locator("[data-testid=place-ws-pick]", { hasText: other.name }).click();
  ok(await isCrumb(page, other.name) && await until(page, (id) => document.querySelector("[data-testid=dock-ws]")?.dataset.ws === id, other.id), `${tag} picked, it is that one's, in the dock too`);
  // From each card.
  await page.click("[data-testid=nav-view-workspaces]");
  await rowIn(page, "Everything", root).click();
  ok(await isCrumb(page, "Everything"), `${tag} from one card, that one`);
  await rowIn(page, other.name, root).click();
  ok(await isCrumb(page, other.name), `${tag} from the other, the other`);
  // The row's menu and its ▸ are that card's too; a place reached from Home keeps it.
  await rowIn(page, "Everything", root).click({ button: "right" });
  await page.locator(".ctxmenu .pop-item", { hasText: /^Enter/ }).first().click();
  ok(await isCrumb(page, "Everything"), `${tag} its menu's Enter is from that card`);
  await rowIn(page, other.name, root).hover();
  await rowIn(page, other.name, root).locator("button").first().click();
  ok(await isCrumb(page, other.name), `${tag} so is its ▸`);
  // Back is the place, with the card it was last clicked in.
  // The history folds steps under 600 ms into one visit: stay a moment on each.
  await page.waitForTimeout(800);
  await page.click(".home-item");
  await page.waitForTimeout(800);
  await back(page);
  ok(await isCrumb(page, other.name), `${tag} Back to it keeps its workspace`);
  // It leaves. (This place's dock has not been open yet: a place keeps its own.)
  await rail(page).click();
  await page.waitForSelector("[data-testid=dock-ws]");
  await call(page, "workspace_set_projects", { id: other.id, roots: [] });
  await refocus(page);
  ok(await isCrumb(page, "Everything"), `${tag} the project leaves it: the place stays, with the one that still holds it`);
  await call(page, "workspace_set_projects", { id: all.id, roots: all.projects.map((p) => p.root).filter((r) => r !== root) });
  await refocus(page);
  ok(await until(page, () => !document.querySelector("[data-testid=place-ws]")) && await page.locator(".topbar .slug").first().isVisible() && await rail(page).count() === 0, `${tag} in none: the place stays, the crumb and the tab go`);
  ok(await until(page, () => !!document.querySelector(".dock .dock-body") && !document.querySelector(".dock .wsd")), `${tag} …and the dock shows Files`);
}

async function dock(page, tag, name) {
  const { all, other, root } = await boot(page);
  await call(page, "workspace_set_projects", { id: other.id, roots: [] });
  const plan = (await call(page, "plan_make", { ideas: [JELLY], title: "Jellyfin plan" })).id;
  await call(page, "workspace_set_plan", { id: all.id, plan });
  for (const i of [JELLY, GPU]) await call(page, "workspace_link_idea", { id: all.id, idea: i });
  await refocus(page); await page.waitForTimeout(300);
  await rowIn(page, "Everything", root).click();
  await isCrumb(page, "Everything");
  await rail(page).click();
  ok(await until(page, () => document.querySelector("[data-testid=dock-ws]")?.dataset.tab === "memory" && /What this workspace is for/.test(document.querySelector("[data-testid=dock-ws-read]")?.innerText ?? "")), `${tag} the dock tab reads the workspace's memory`);
  ok(await page.locator("[data-testid=dock-ws-tab]").evaluateAll((b) => b.map((x) => x.textContent).join()) === "Memory,Plan,Ideas 2", `${tag} Memory, Plan, Ideas`);
  ok(await page.evaluate(() => !document.querySelector("[data-testid=dock-ws] .cm-content, [data-testid=dock-ws] textarea, [data-testid=dock-ws] [data-testid=ideas-mode-edit], [data-testid=ws-chat-col]") && !performance.getEntriesByType("resource").some((r) => /codemirror|\/doc\/editor\.ts/.test(r.name))), `${tag} to read: no editor, none fetched, and no chat`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsp-dock-${name}.png` });
  // ── memory ──
  await page.locator("[data-testid=dock-ws-read] a", { hasText: "NAS layout" }).click();
  ok(await until(page, () => document.querySelector("[data-testid=dock-ws-read]")?.dataset.doc === "topics/nas-layout.md"), `${tag} a link opens in place`);
  ok(await until(page, () => { const i = document.querySelector("[data-testid=dock-ws-read] img"); return i && i.naturalWidth > 0; }), `${tag} with its image`);
  await page.click("[data-testid=dock-ws-back]");
  ok(await until(page, () => document.querySelector("[data-testid=dock-ws-read]")?.dataset.doc === "MEMORY.md"), `${tag} and back`);
  await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# Memory\n\nWRITTEN MEANWHILE [gone](no.md)\n"), all.id);
  ok(await until(page, () => /WRITTEN MEANWHILE/.test(document.querySelector("[data-testid=dock-ws-read]")?.innerText ?? "")) && await until(page, () => !!document.querySelector("[data-testid=dock-ws-read] a.gone")), `${tag} what an agent writes is read as it lands, a missing link dim`);
  await page.click("[data-testid=dock-ws-files]");
  ok(await page.locator("[data-testid=dock-ws] [data-testid=memory-file]").count() >= 5 && await page.locator("[data-testid=dock-ws] [data-testid=memory-new]").count() === 0, `${tag} its files, with no New file`);
  await page.click("[data-testid=dock-ws] [data-testid=memory-file][data-path='files/rack.svg']");
  ok(await until(page, () => document.querySelector("[data-testid=memory-preview] img")?.naturalWidth > 0), `${tag} an image has its preview`);
  await page.click("[data-testid=dock-ws-files]");
  await page.click("[data-testid=dock-ws] [data-testid=memory-file][data-path='topics/nas-layout.md']");
  // ── plan, ideas ──
  await page.click("[data-testid=dock-ws-tab][data-tab=plan]");
  ok(await until(page, () => /Plan: Jellyfin plan/.test(document.querySelector("[data-testid=dock-ws-read]")?.innerText ?? "")), `${tag} the plan`);
  await page.locator("[data-testid=dock-ws-read] a").first().click();
  ok(await until(page, (id) => document.querySelector("[data-testid=dock-ws]")?.dataset.tab === "ideas" && document.querySelector("[data-testid=dock-ws-read]")?.dataset.doc === id, JELLY), `${tag} its link to an idea is read on Ideas`);
  await page.click("[data-testid=dock-ws-ideas]");
  await page.click(`[data-testid=dock-ws-idea][data-idea='${GPU}']`);
  ok(await until(page, (id) => document.querySelector("[data-testid=dock-ws-read]")?.dataset.doc === id, GPU), `${tag} another idea`);
  // ── Edit in ──
  await page.waitForTimeout(800);
  ok(/^Edit in Everything ▸$/.test(await page.locator("[data-testid=dock-ws-edit]").innerText()), `${tag} Edit in Everything`);
  await page.click("[data-testid=dock-ws-edit]");
  ok(await until(page, (id) => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === "ideas" && document.querySelector("[data-testid=main-doc]")?.dataset.doc === id && !!document.querySelector("[data-testid=main-doc] .cm-content"), GPU), `${tag} …opens it in the workspace view, in Edit`);
  await page.waitForTimeout(800);
  await back(page);
  ok(await isCrumb(page, "Everything") && await until(page, () => document.querySelector("[data-testid=dock-ws]")?.dataset.tab === "ideas"), `${tag} and Back is the place, its dock as it was`);
  await page.click("[data-testid=dock-ws-tab][data-tab=memory]");
  await page.click("[data-testid=dock-ws-edit]");
  ok(await until(page, () => document.querySelector("[data-testid=main-doc]")?.dataset.doc === "topics/nas-layout.md" && !!document.querySelector("[data-testid=main-doc] .cm-content")), `${tag} a memory file too, the one that was read`);
  // Typed there, read here.
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type("\nTYPED THERE");
  await rowIn(page, "Everything", root).click();
  ok(await until(page, () => /TYPED THERE/.test(document.querySelector("[data-testid=dock-ws-read]")?.innerText ?? "")), `${tag} what was typed there is read here`);
  // ── the crumb ──
  await page.click("[data-testid=place-ws]");
  ok(await until(page, () => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === "memory" && document.querySelector("[data-testid=main-doc]")?.dataset.doc === "topics/nas-layout.md"), `${tag} the crumb goes back to where you were in it`);
  // With a chat: it is where you were, and never in the place.
  await card(page, "Everything").locator("[data-testid=ws-chat-row]").first().click();
  await page.waitForSelector("[data-testid=ws-chat-col]");
  await rowIn(page, "Everything", root).click();
  ok(await until(page, () => !!document.querySelector("[data-testid=place-ws]")) && !(await page.locator("[data-testid=ws-chat-col]").isVisible().catch(() => false)), `${tag} no workspace chat inside a place's view`);
  await page.click("[data-testid=place-ws]");
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-col]")), `${tag} …and the crumb brings it back`);
  // ── Float ── (the hand-off is documents-float-browser.mjs's)
  await rowIn(page, "Everything", root).click();
  for (const t of ["memory", "plan"]) {
    await page.click(`[data-testid=dock-ws-tab][data-tab=${t}]`);
    ok(await page.locator("[data-testid=dock-ws-float]").count() === 1, `${tag} ${t} floats from here`);
  }
}

async function betas(page, tag) {
  for (const [on, tabs] of [[["beta_ideas"], "Plan,Ideas"], [["beta_workspace_chats"], "Memory"], [[], null]]) {
    const p = await page.context().newPage();
    await boot(p, on);
    await p.locator("[data-testid=ws-group] li.row[data-slug]").nth(1).click();
    await p.waitForSelector(".topbar .slug");
    await p.waitForTimeout(300);
    if (tabs === null) { ok(!(await p.$("[data-testid=place-ws]")) && await rail(p).count() === 0, `${tag} with neither beta a place's bar and dock are as they were`); await p.close(); continue; }
    if ((await crumb(p)) !== "Everything") { await p.click("[data-testid=place-ws]"); await p.locator("[data-testid=place-ws-pick]").first().click(); }
    await rail(p).click();
    ok(await until(p, () => !!document.querySelector("[data-testid=dock-ws]")) && (await p.locator("[data-testid=dock-ws-tab]").evaluateAll((b) => b.map((x) => x.textContent).join())) === tabs, `${tag} ${on[0]} alone: ${tabs}`);
    await p.close();
  }
}

async function relaunch(page, tag) {
  const { other, root } = await boot(page, undefined, { restore_last: true });
  await page.waitForTimeout(800);
  await card(page, "Everything").locator(`[data-project-root='${root}'] li.row[data-slug]`).nth(1).click();
  await call(page, "workspace_set_projects", { id: other.id, roots: [root] });
  await refocus(page);
  await isCrumb(page, "Everything");
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem("wt.place.ws") ?? "{}"));
  ok(Object.values(stored).length === 1, `${tag} the card a place was clicked in is kept (${JSON.stringify(stored)})`);
  await page.reload();
  await page.waitForSelector("[data-testid=ws-group]");
  await call(page, "workspace_set_projects", { id: other.id, roots: [root] });
  await refocus(page);
  await page.waitForTimeout(400);
  const slug = Object.keys(stored)[0].split("|").pop();
  await page.click("[data-testid=nav-view-workspaces]");
  await page.keyboard.press("Meta+k");
  await page.fill(".qs-input", slug);
  await page.keyboard.press("Enter");
  ok(await isCrumb(page, "Everything"), `${tag} after a relaunch, in two workspaces and reached by ⌘K, it is still that one`);
}

async function layout(page, tag, name) {
  const { all, other, root } = await boot(page);
  await call(page, "workspace_set_projects", { id: other.id, roots: [] });
  await call(page, "workspace_link_idea", { id: all.id, idea: JELLY });
  await refocus(page); await page.waitForTimeout(300);
  await rowIn(page, "Everything", root).click();
  await rail(page).click();
  await page.waitForSelector("[data-testid=dock-ws-read]");
  for (const w of [1600, 1440, 1200]) {
    await page.setViewportSize({ width: w, height: 800 });
    await page.waitForTimeout(250);
    const bad = await page.evaluate(() => [...document.querySelectorAll("[data-testid=dock-ws] button:not(:disabled), [data-testid=place-ws], .topbar .controls button")].filter((b) => { const q = b.getBoundingClientRect(); if (!q.width) return false; const hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2); return !(hit && (b === hit || b.contains(hit))); }).map((b) => b.dataset.testid || b.title));
    const m = await page.evaluate(() => { const d = document.querySelector(".dock").getBoundingClientRect(), r = document.querySelector("[data-testid=dock-ws]").getBoundingClientRect(), t = document.querySelector("[data-testid=dock-ws-read]").getBoundingClientRect(); return { in: r.left >= d.left - 1 && r.right <= d.right + 1 && t.bottom <= innerHeight + 1, h: t.height, bar: document.querySelector(".topbar").getBoundingClientRect().height, split: [...document.querySelectorAll(".topbar .s")].filter((e) => e.getBoundingClientRect().height > 20).length, name: (() => { const n = document.querySelector(".topbar .slug"); return [n.clientWidth, n.scrollWidth, document.querySelector(".topbar-proj-name").clientWidth, Math.round(document.querySelector("[data-testid=place-ws]").getBoundingClientRect().width)]; })() }; });
    ok(m.split === 0 && (w < 1440 || m.name[0] >= m.name[1] - 1), `${tag} ${w}px: the crumb gives way before the place's name, and no count breaks (${JSON.stringify(m.name)})`);
    ok(bad.length === 0 && m.in && m.h > 200 && m.bar < 60, `${tag} ${w}px: reachable, inside the dock, one bar${bad.length ? `: ${bad}` : ""} (${JSON.stringify(m)})`);
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  for (const theme of THEMES) {
    await page.evaluate((t) => { if (t === "tokyo-night") delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t; }, theme);
    const low = await page.evaluate(() => {
      const parse = (c) => { let x = /^rgba?\(([^)]+)\)/.exec(c); if (x) { const p = x[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; } x = /^color\(srgb ([^)]+)\)/.exec(c); if (x) { const p = x[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; } return [0, 0, 0, 0]; };
      const over = (a, b) => [0, 1, 2].map((i) => a[i] * a[3] + b[i] * (1 - a[3])).concat(1);
      const bgOf = (el) => { const st = []; for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) st.push(c); if (c[3] >= 1) break; } let acc = [255, 255, 255, 1]; for (const c of st.reverse()) acc = over(c, acc); return acc; };
      const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
      const ratio = (el) => { const bg = bgOf(el); const [a, b] = [lum(over(parse(getComputedStyle(el).color), bg)), lum(bg)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
      return [...document.querySelectorAll(".wsd-name, .wsd-files, .wsd-doc .ctrl, [data-testid=place-ws] .wsf-name-t")].filter((e) => e.textContent.trim() && e.getBoundingClientRect().width).map((e) => [e.className + ": " + e.textContent.trim().slice(0, 24), ratio(e)]).filter(([, r]) => r < 4.5);
    });
    ok(low.length === 0, `${tag} ${theme}: its words read ≥ 4.5:1${low.length ? `: ${low.map(([a, r]) => `${a} ${r.toFixed(2)}`).join("; ")}` : ""}`);
  }
  // The dock on the left.
  const p = await page.context().newPage();
  const b = await boot(p, undefined, { dock_side: "left", places_side: "right" });
  await call(p, "workspace_set_projects", { id: b.other.id, roots: [] });
  await refocus(p); await p.waitForTimeout(300);
  await rowIn(p, "Everything", b.root).click();
  await rail(p).click();
  ok(await until(p, () => { const d = document.querySelector("[data-testid=dock-ws]"), m = document.querySelector(".main"); return d && m && d.getBoundingClientRect().right <= m.getBoundingClientRect().left + 1; }), `${tag} with the dock on the left, it is on the left`);
  if (SHOTS) await p.screenshot({ path: `${SHOTS}/wsp-left-${name}.png` });
  await p.close();
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [which, dock, betas, relaunch, layout]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    page.on("console", (m) => { if (m.type() === "error" && /Maximum update depth|Cannot update a component|unique "key"/.test(m.text())) errors.push(m.text().slice(0, 200)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag, name); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 12).join(" ")}`); if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsp-${name}-${part.name}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nworkspace-place-browser: ${failed} failure(s)` : "\nworkspace-place-browser: all good");
process.exit(failed ? 1 : 0);
