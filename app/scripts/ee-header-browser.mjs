// Optional browser gate for "Show the Everything Else header" (Settings →
// Layout, `nav_ee_header`), in Chromium AND WebKit, against the mock.
// ee-header-check.mjs holds the key and the wiring; this is what the nav
// DRAWS in each position and what its controls then do.
//
//   VITE_MOCK=1 vite --port 1489 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     EE_MOCK_URL=http://localhost:1489 node app/scripts/ee-header-browser.mjs
//
//   - on (the default): the header, its count, its fold, as before;
//   - off: no header, no card chrome, the same projects in the same order,
//     below the named workspaces, each row reachable (elementFromPoint), its
//     project header sticky at the top; the right-click menu is All projects';
//   - off over a stored fold: the projects show; on again: the fold is back;
//   - the checkbox in Settings flips it live, and the choice survives a reload;
//   - with no named workspace the header shows in BOTH positions;
//   - a drag out of a workspace onto the bare area lights nothing and does
//     nothing (it never did: Everything Else is no drop target), and a drag of
//     a bare-area project onto a workspace adds it, as from under the header.
// Screenshots (both positions, 240px and 300px nav, dark and light) go to
// EE_SHOTS (default: the place's .worktrees-attachments/agent/).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const { chromium, webkit } = pw.chromium ? pw : pw.default;
const BASE = process.env.EE_MOCK_URL ?? "http://localhost:1489";
const SHOTS = process.env.EE_SHOTS ?? fileURLToPath(new URL("../../.worktrees-attachments/agent/", import.meta.url));
fs.mkdirSync(SHOTS, { recursive: true });
const only = process.argv.find((a) => a.startsWith("--engine="))?.slice(9);

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

const SEED = { places_side: "left", dock_side: "left", settings_rev: 4, nav_pinned: true, nav_view: "workspaces" };
async function open(browser, extra = {}, query = "workspaces=1", viewport = { width: 1280, height: 1400 }) {
  const page = await browser.newPage({ viewport });
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { ...SEED, ...extra });
  await page.goto(`${BASE}/?${query}`);
  await page.waitForSelector("[data-testid=ee-group]");
  await page.waitForTimeout(200);
  if (query.includes("workspaces=1")) await freeOne(page);
  return page;
}
/** The mock's Everything Else holds only a "repo gone" project, which has no
 *  places: take the first workspace's first project out of it (it is in no
 *  other), so the group has real rows to measure. */
async function freeOne(page) {
  await page.evaluate(() => {
    const h = document.querySelector("[data-testid=ws-group] .ws-kids > .project[data-project-root] .project-h");
    const r = h.getBoundingClientRect();
    h.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 30, clientY: r.top + 5 }));
  });
  await page.waitForTimeout(100);
  await page.click("[data-testid=ctx-ws-remove]");
  await page.waitForTimeout(250);
}
/** What the nav shows of Everything Else. */
const ee = (page) => page.evaluate(() => {
  const g = document.querySelector("[data-testid=ee-group]");
  if (!g) return null;
  const cs = getComputedStyle(g);
  const named = [...document.querySelectorAll("[data-testid=ws-group]")];
  const rows = [...g.querySelectorAll(".project[data-project-root]")];
  return {
    header: !!g.querySelector("[data-testid=ee-header]"),
    headerText: g.querySelector("[data-testid=ee-header]")?.textContent ?? null,
    bare: g.classList.contains("bare"),
    border: cs.borderTopWidth, bg: cs.backgroundColor,
    kidsRule: g.querySelector(".ws-kids") ? getComputedStyle(g.querySelector(".ws-kids")).borderTopWidth : null,
    projects: rows.map((r) => r.dataset.projectRoot),
    places: g.querySelectorAll("li.row[data-slug]").length,
    afterNamed: named.every((n) => n.compareDocumentPosition(g) & Node.DOCUMENT_POSITION_FOLLOWING),
    stickyTop: rows[0] ? getComputedStyle(rows[0].querySelector(".project-h")).top : null,
    anyTitle: g.textContent.includes("Everything Else"),
  };
});
const reachable = (page, sel) => page.evaluate((s) => [...document.querySelectorAll(s)].map((el) => {
  el.scrollIntoView({ block: "nearest" });
  const r = el.getBoundingClientRect();
  const hit = document.elementFromPoint(r.left + Math.min(60, r.width / 2), r.top + r.height / 2);
  return !!hit && (hit === el || el.contains(hit));
}), sel);
const pointer = (page, type, sel, dy = 0) => page.evaluate(async ([t, s, d]) => {
  const el = document.querySelector(s);
  const r = el.getBoundingClientRect();
  const x = r.left + Math.min(40, r.width / 2), y = r.top + r.height / 2 + d;
  (t === "pointerdown" ? el : window).dispatchEvent(new PointerEvent(t, { bubbles: true, clientX: x, clientY: y, button: 0, buttons: t === "pointerup" ? 0 : 1, pointerId: 1 }));
  await new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(res)));
}, [type, sel, dy]);
const wsProjects = (page) => page.evaluate(() => Object.fromEntries([...document.querySelectorAll("[data-testid=ws-group]")].map((g) => [
  g.querySelector(".ws-name")?.textContent, [...g.querySelectorAll(".ws-kids > .project[data-project-root]")].map((p) => p.dataset.projectRoot)])));
const lit = (page) => page.$$eval("[data-testid=ws-group].drop-on, [data-testid=ee-group].drop-on", (g) => g.length);

async function run(engine, name) {
  const browser = await engine.launch();
  const tag = `[${name}]`;
  try {
    // ── on: the default, as before ──
    let page = await open(browser);
    const on = await ee(page);
    ok(on && on.header && !on.bare && on.projects.length > 1 && on.places > 0, `${tag} on (default): the header is drawn over ${on?.projects.length} projects`);
    ok(/Everything Else\s*\d+/.test(on?.headerText ?? "") && on.headerText.includes(String(on.projects.length)), `${tag} …with its title and count: ${JSON.stringify(on?.headerText)}`);
    ok(on.border !== "0px" && on.kidsRule !== "0px", `${tag} …in its card (border ${on.border}, rule ${on.kidsRule})`);
    await page.click("[data-testid=ee-header] .ws-name");
    await page.waitForTimeout(150);
    ok((await ee(page)).projects.length === 0, `${tag} …and its fold folds`);
    await page.click("[data-testid=ee-header] .ws-name");
    await page.waitForTimeout(150);

    // ── the checkbox in Settings flips it live ──
    await page.click("[data-track=settings]");
    await page.click(".settings-cat[data-cat=layout]");
    await page.waitForSelector(".settings-body[data-page=layout]");
    const box = page.locator("label.setting-check", { hasText: "Show the Everything Else header" }).locator("input[type=checkbox]");
    ok(await box.count() === 1 && await box.isChecked(), `${tag} Settings → Layout: "Show the Everything Else header", ticked`);
    await box.click();
    await page.waitForTimeout(150);
    ok(!(await box.isChecked()), `${tag} …unticked`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(200);
    const off = await ee(page);
    ok(off && !off.header && off.bare && !off.anyTitle, `${tag} off: no header and no "Everything Else" anywhere in the group`);
    ok(off.projects.join() === on.projects.join() && off.places === on.places && off.places > 0, `${tag} …the same projects, in the same order, every place (${off.places})`);
    ok(off.afterNamed, `${tag} …below the named workspaces`);
    ok(off.border === "0px" && off.kidsRule === "0px" && /rgba\(0, 0, 0, 0\)|transparent/.test(off.bg), `${tag} …with no card: border ${off.border}, rule ${off.kidsRule}, background ${off.bg}`);
    ok(off.stickyTop === "0px", `${tag} …its projects' headers stick at the top (${off.stickyTop})`);
    ok((await reachable(page, "[data-testid=ee-group] .project-h")).every(Boolean), `${tag} …each project header is what is under it (elementFromPoint)`);
    ok((await reachable(page, "[data-testid=ee-group] li.row[data-slug]")).every(Boolean), `${tag} …and each place row`);
    // the rows' menu is All projects' (no "Remove from workspace")
    await page.evaluate(() => {
      const h = document.querySelector("[data-testid=ee-group] .project-h");
      const r = h.getBoundingClientRect();
      h.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 30, clientY: r.top + 5 }));
    });
    await page.waitForTimeout(100);
    const menu = await page.evaluate(() => document.querySelector(".ctxmenu")?.textContent ?? "");
    ok(menu.length > 0 && !/Remove from workspace/.test(menu), `${tag} …its right-click menu is All projects' (no Remove from workspace)`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(100);
    // the filter's Enter selects the first row the nav shows; a bare row can be it
    // (keyboard nav reads the rendered rows, so nothing hides them from it)

    // a drag of a workspace's project onto the bare area: nothing
    const before = await wsProjects(page);
    const from = "[data-testid=ws-group] .ws-kids > .project[data-project-root] .project-h";
    await page.$eval(".nav-scroll", (n) => { n.scrollTop = 0; });
    await pointer(page, "pointerdown", from);
    await pointer(page, "pointermove", from, 6);
    await pointer(page, "pointermove", "[data-testid=ee-group] .project-h");
    ok(await lit(page) === 0 && (await page.$$(".drop-gap.wsp")).length === 0, `${tag} a workspace's project dragged over the bare area: nothing lights, no gap`);
    await pointer(page, "pointerup", "[data-testid=ee-group] .project-h");
    await page.waitForTimeout(250);
    ok(JSON.stringify(await wsProjects(page)) === JSON.stringify(before), `${tag} …and the drop changes nothing`);
    // a bare-area project onto a named workspace: added
    const root = (await ee(page)).projects[0];
    const src = `[data-testid=ee-group] .project[data-project-root="${root}"] .project-h`;
    await pointer(page, "pointerdown", src);
    await pointer(page, "pointermove", src, 6);
    await pointer(page, "pointermove", "[data-testid=ws-group] .ws-h");
    ok(await lit(page) === 1, `${tag} a bare-area project dragged onto a workspace lights it`);
    await pointer(page, "pointerup", "[data-testid=ws-group] .ws-h");
    await page.waitForTimeout(300);
    const after = await wsProjects(page);
    ok(Object.values(after)[0].includes(root) && !(await ee(page))?.projects.includes(root), `${tag} …and the drop adds it, out of the bare area`);

    // the choice survives a reload (the blob)
    await page.reload();
    await page.waitForSelector("[data-testid=ee-group]");
    await page.waitForTimeout(200);
    ok(!(await ee(page)).header, `${tag} off survives a reload`);
    await page.close();

    // ── off over a stored fold: the projects show; on again: the fold is back ──
    page = await open(browser, { nav_ee_header: false, ws_collapsed: { "everything-else": true } });
    const folded = await ee(page);
    ok(!folded.header && folded.projects.length > 0, `${tag} off with Everything Else folded: its ${folded.projects.length} projects show`);
    await page.click("[data-track=settings]");
    await page.click(".settings-cat[data-cat=layout]");
    await page.locator("label.setting-check", { hasText: "Show the Everything Else header" }).locator("input").click();
    await page.keyboard.press("Escape");
    await page.waitForTimeout(200);
    const back = await ee(page);
    ok(back.header && back.projects.length === 0, `${tag} …on again: the header is back, folded as it was stored`);
    await page.close();

    // ── no named workspace: the header in both positions ──
    for (const v of [true, false]) {
      page = await open(browser, { nav_ee_header: v }, "");
      const g = await ee(page);
      const empty = await page.$$("[data-testid=ws-empty]");
      ok(g.header && !g.bare && g.projects.length > 0 && empty.length === 1, `${tag} no named workspace, setting ${v ? "on" : "off"}: the "No workspaces yet" box and the header over every project, as today`);
      await page.close();
    }

    // ── screenshots: 240px and 300px nav, dark and light, both positions ──
    if (name === "webkit") {
      for (const theme of ["tokyo-night", "tokyo-day"]) {
        for (const w of [240, 300]) {
          for (const v of [true, false]) {
            page = await open(browser, { nav_ee_header: v, nav_width: w, theme }, "workspaces=1", { width: 1100, height: 900 });
            if (w === 240) {
              const fit = await page.evaluate(() => {
                const g = document.querySelector("[data-testid=ee-group]").getBoundingClientRect();
                const nav = document.querySelector(".nav-scroll").getBoundingClientRect();
                return g.left >= nav.left - 0.5 && g.right <= nav.right + 0.5;
              });
              ok(fit, `${tag} ${theme} ${w}px ${v ? "on" : "off"}: the group fits the nav column`);
              ok((await reachable(page, "[data-testid=ee-group] .project-h")).every(Boolean), `${tag} ${theme} ${w}px ${v ? "on" : "off"}: every project header is reachable`);
            }
            await page.$eval("[data-testid=ee-group]", (g) => g.scrollIntoView({ block: "center" }));
            await page.waitForTimeout(100);
            const nav = await page.$(".nav");
            await (nav ?? page).screenshot({ path: path.join(SHOTS, `ee-header-${v ? "on" : "off"}-${w}-${theme}.png`) });
            await page.close();
          }
        }
      }
    }
  } finally {
    await browser.close();
  }
}

for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  if (only && only !== name) continue;
  await run(engine, name);
}
console.log(failed ? `\n${failed} failed` : "\nall passed");
process.exit(failed ? 1 : 0);
